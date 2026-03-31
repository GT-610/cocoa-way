use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::NonNull;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_foundation::{NSString, NSError};
use objc2_metal::*;
use objc2_quartz_core::{CAMetalLayer, CAMetalDrawable};
use smithay::reexports::wayland_server::backend::ObjectId;
use winit::window::Window;

// All three shaders share one vertex function; each has its own fragment function.
const SHADER_SOURCE: &str = r#"
#include <metal_stdlib>
using namespace metal;

struct VertexOut {
    float4 position [[position]];
    float2 texcoord;
};

struct Rect { float x, y, w, h; };

vertex VertexOut vert_main(uint vid [[vertex_id]],
                           constant Rect& rect [[buffer(0)]]) {
    // Z-pattern (BL, BR, TL, TR) — the N-pattern (BL,BR,TR,TL) leaves
    // a left-diamond region uncovered between the two triangles.
    float2 pos[4] = {float2(0,0), float2(1,0), float2(0,1), float2(1,1)};
    float2 uv[4]  = {float2(0,1), float2(1,1), float2(0,0), float2(1,0)};
    VertexOut out;
    out.position = float4(rect.x + pos[vid].x * rect.w,
                          rect.y + pos[vid].y * rect.h, 0.0, 1.0);
    out.texcoord = uv[vid];
    return out;
}

fragment float4 frag_blit(VertexOut in [[stage_in]],
                          texture2d<float> tex [[texture(0)]]) {
    constexpr sampler s(filter::linear, address::clamp_to_edge);
    return tex.sample(s, in.texcoord);
}

fragment float4 frag_solid(VertexOut in [[stage_in]],
                            constant float4& color [[buffer(1)]]) {
    return color;
}

fragment float4 frag_border(VertexOut in [[stage_in]],
                             constant float4& color [[buffer(1)]],
                             constant float&  width [[buffer(2)]]) {
    float d = min(min(in.texcoord.x, 1.0-in.texcoord.x),
                  min(in.texcoord.y, 1.0-in.texcoord.y));
    if (d < width) return color;
    discard_fragment();
    return float4(0);
}

fragment float4 frag_shadow(VertexOut in [[stage_in]],
                             constant float4& color [[buffer(1)]],
                             constant float&  sigma [[buffer(2)]]) {
    float fx = smoothstep(0.0, sigma, in.texcoord.x)
             * smoothstep(0.0, sigma, 1.0-in.texcoord.x);
    float fy = smoothstep(0.0, sigma, in.texcoord.y)
             * smoothstep(0.0, sigma, 1.0-in.texcoord.y);
    return color * (1.0 - fx * fy);
}
"#;

// Rect packed as four floats for the vertex buffer (NDC space)
#[repr(C)]
struct RectUniform { x: f32, y: f32, w: f32, h: f32 }

// Safety: caller ensures the reference outlives the GPU call.
#[inline(always)]
unsafe fn as_bytes<T>(v: &T) -> NonNull<c_void> {
    unsafe { NonNull::new_unchecked(v as *const T as *mut c_void) }
}
#[inline(always)]
unsafe fn slice_bytes<T>(v: &[T]) -> NonNull<c_void> {
    unsafe { NonNull::new_unchecked(v.as_ptr() as *mut c_void) }
}

struct FrameState {
    drawable:       Retained<ProtocolObject<dyn CAMetalDrawable>>,
    command_buffer: Retained<ProtocolObject<dyn MTLCommandBuffer>>,
    encoder:        Retained<ProtocolObject<dyn MTLRenderCommandEncoder>>,
}

// Per-surface texture cache entry
struct CachedTexture {
    texture:   Retained<ProtocolObject<dyn MTLTexture>>,
    buffer_id: ObjectId,
    tex_w:     i32,
    tex_h:     i32,
}

pub struct MetalRenderer {
    pub window:   Window,
    pub width:    u32,
    pub height:   u32,
    device:        Retained<ProtocolObject<dyn MTLDevice>>,
    layer:         Retained<CAMetalLayer>,
    command_queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    blit_pipeline:   Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
    solid_pipeline:  Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
    border_pipeline: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
    shadow_pipeline: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
    clear_color: (f64, f64, f64, f64),
    // Set for the duration of one rendered frame
    frame: RefCell<Option<FrameState>>,
    // surface ObjectId → cached GPU texture
    texture_cache: RefCell<HashMap<ObjectId, CachedTexture>>,
    // text label → cached GPU texture (key = "label|r,g,b,a|font_size")
    text_cache: RefCell<HashMap<String, (Retained<ProtocolObject<dyn MTLTexture>>, u32, u32)>>,
}

impl MetalRenderer {
    pub fn new(window: Window) -> Result<Self, String> {
        unsafe {
            // ── 1. Get Metal device ──────────────────────────────────────────
            let device_ptr = MTLCreateSystemDefaultDevice();
            let device: Retained<ProtocolObject<dyn MTLDevice>> =
                Retained::from_raw(device_ptr)
                    .ok_or("No Metal device available")?;
            log::info!("Metal device: {:?}", device.name());

            // ── 2. Create CAMetalLayer and attach to the NSView ──────────────
            let layer = CAMetalLayer::new();
            layer.setDevice(Some(&device));
            layer.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
            layer.setFramebufferOnly(false); // needed for texture reads if required
            let scale = window.scale_factor();
            let () = objc2::msg_send![&layer, setContentsScale: scale as f64];

            // Attach the Metal layer to the window's NSView using raw ObjC sends.
            // winit 0.29 on macOS exposes an AppKitWindowHandle { ns_view }.
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            let ns_view: *mut objc2::runtime::AnyObject =
                match window.window_handle().map_err(|e| e.to_string())?.as_raw() {
                    RawWindowHandle::AppKit(h) => h.ns_view.as_ptr() as *mut _,
                    _ => return Err("Non-AppKit window handle".into()),
                };
            let () = objc2::msg_send![ns_view, setWantsLayer: true];
            let () = objc2::msg_send![ns_view, setLayer: &*layer];

            let size = window.inner_size();
            let cg_size = objc2_foundation::CGSize {
                width:  size.width  as f64,
                height: size.height as f64,
            };
            layer.setDrawableSize(cg_size);

            // ── 3. Command queue ─────────────────────────────────────────────
            let command_queue = device.newCommandQueue()
                .ok_or("Failed to create command queue")?;

            // ── 4. Compile shaders ───────────────────────────────────────────
            let src = NSString::from_str(SHADER_SOURCE);
            let library = device
                .newLibraryWithSource_options_error(&src, None)
                .map_err(|e: Retained<NSError>| format!("Shader compile error: {}", e.localizedDescription()))?;

            let vert = library.newFunctionWithName(&NSString::from_str("vert_main"))
                .ok_or("Missing vert_main")?;
            let frag_blit   = library.newFunctionWithName(&NSString::from_str("frag_blit"))  .ok_or("Missing frag_blit")?;
            let frag_solid  = library.newFunctionWithName(&NSString::from_str("frag_solid")) .ok_or("Missing frag_solid")?;
            let frag_border = library.newFunctionWithName(&NSString::from_str("frag_border")).ok_or("Missing frag_border")?;
            let frag_shadow = library.newFunctionWithName(&NSString::from_str("frag_shadow")).ok_or("Missing frag_shadow")?;

            // ── 5. Pipeline states ───────────────────────────────────────────
            // blit is OPAQUE — tile content always replaces the background completely.
            // Using blending=false prevents any transparent holes from showing through.
            let blit_pipeline   = make_pipeline(&device, &vert, &frag_blit,   false)?;
            let solid_pipeline  = make_pipeline(&device, &vert, &frag_solid,  true)?;
            let border_pipeline = make_pipeline(&device, &vert, &frag_border, true)?;
            let shadow_pipeline = make_pipeline(&device, &vert, &frag_shadow, true)?;

            Ok(Self {
                width:  size.width,
                height: size.height,
                window,
                device,
                layer,
                command_queue,
                blit_pipeline,
                solid_pipeline,
                border_pipeline,
                shadow_pipeline,
                clear_color: (0.1, 0.1, 0.15, 1.0),
                frame: RefCell::new(None),
                texture_cache: RefCell::new(HashMap::new()),
                text_cache: RefCell::new(HashMap::new()),
            })
        }
    }

    pub fn set_scale_factor(&mut self, scale: f64) {
        unsafe {
            let () = objc2::msg_send![&self.layer, setContentsScale: scale];
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 { return; }
        self.width  = width;
        self.height = height;
        unsafe {
            self.layer.setDrawableSize(objc2_foundation::CGSize {
                width:  width  as f64,
                height: height as f64,
            });
        }
    }

    /// Begin a frame: set clear colour, acquire drawable, start the render pass.
    pub fn clear(&self, r: f32, g: f32, b: f32, a: f32) {
        let mut frame_slot = self.frame.borrow_mut();
        // Drop any leftover frame state from a previous incomplete frame.
        *frame_slot = None;

        let Some(drawable) = (unsafe { self.layer.nextDrawable() }) else {
            log::warn!("Metal: nextDrawable returned nil");
            return;
        };
        let Some(cmd_buf) = (unsafe { self.command_queue.commandBuffer() }) else {
            log::warn!("Metal: commandBuffer returned nil");
            return;
        };

        let rp = unsafe {
            let desc = MTLRenderPassDescriptor::new();
            let ca = desc.colorAttachments().objectAtIndexedSubscript(0);
            let texture = drawable.texture();
            ca.setTexture(Some(&texture));
            ca.setLoadAction(MTLLoadAction::Clear);
            ca.setStoreAction(MTLStoreAction::Store);
            ca.setClearColor(MTLClearColor { red: r as f64, green: g as f64, blue: b as f64, alpha: a as f64 });
            desc
        };

        let Some(encoder) = (unsafe { cmd_buf.renderCommandEncoderWithDescriptor(&rp) }) else {
            log::warn!("Metal: renderCommandEncoder returned nil");
            return;
        };

        *frame_slot = Some(FrameState { drawable, command_buffer: cmd_buf, encoder });
    }

    pub fn swap_buffers(&self) -> Result<(), String> {
        let mut frame_slot = self.frame.borrow_mut();
        let Some(frame) = frame_slot.take() else {
            return Ok(());
        };
        unsafe {
            frame.encoder.endEncoding();
            // CAMetalDrawable implements MTLDrawable; cast via raw ObjC message send.
            let () = objc2::msg_send![&*frame.command_buffer, presentDrawable: &*frame.drawable];
            frame.command_buffer.commit();
        }
        Ok(())
    }

    pub fn request_redraw(&self) {
        self.window.request_redraw();
    }

    // ── Texture cache helpers ─────────────────────────────────────────────────

    pub fn lookup_cached_size(&self, surface_id: &ObjectId, buffer_id: &ObjectId) -> Option<(i32, i32)> {
        self.texture_cache.borrow().get(surface_id).and_then(|e| {
            if &e.buffer_id == buffer_id { Some((e.tex_w, e.tex_h)) } else { None }
        })
    }

    pub fn evict_texture(&self, surface_id: &ObjectId) {
        self.texture_cache.borrow_mut().remove(surface_id);
    }

    /// Render from cache using only surf_id — no buf_id required.
    /// Returns true if a cached texture was found and drawn.
    pub fn draw_from_cache(
        &self,
        surface_id: &ObjectId,
        phys_x: i32,
        phys_y: i32,
        scale: f64,
        viewport_dst: Option<smithay::utils::Size<i32, smithay::utils::Logical>>,
    ) -> bool {
        let cache = self.texture_cache.borrow();
        let Some(entry) = cache.get(surface_id) else { return false; };
        let dest_w = viewport_dst
            .map(|d| (d.w as f64 * scale).round() as i32)
            .unwrap_or(entry.tex_w);
        let dest_h = viewport_dst
            .map(|d| (d.h as f64 * scale).round() as i32)
            .unwrap_or(entry.tex_h);
        if dest_w <= 0 || dest_h <= 0 { return false; }
        let frame_ref = self.frame.borrow();
        let Some(frame) = frame_ref.as_ref() else { return false; };
        let rect = self.to_ndc(phys_x, phys_y, dest_w, dest_h);
        unsafe {
            let enc = &frame.encoder;
            enc.setRenderPipelineState(&self.blit_pipeline);
            enc.setVertexBytes_length_atIndex(
                as_bytes(&rect), std::mem::size_of::<RectUniform>(), 0);
            enc.setFragmentTexture_atIndex(Some(&entry.texture), 0);
            enc.drawPrimitives_vertexStart_vertexCount(MTLPrimitiveType::TriangleStrip, 0, 4);
        }
        true
    }

    // ── Draw calls ───────────────────────────────────────────────────────────

    /// Draw a surface buffer. Pass `tex_w=0, pixels=&[]` on a cache hit.
    pub fn draw_pixels(&self,
                       surface_id: ObjectId, buffer_id: ObjectId,
                       x: i32, y: i32, dest_w: i32, dest_h: i32,
                       tex_w: i32, tex_h: i32, pixels: &[u8]) {
        if dest_w <= 0 || dest_h <= 0 { return; }
        let frame_ref = self.frame.borrow();
        let Some(frame) = frame_ref.as_ref() else { return; };

        let texture = self.get_or_create_texture(surface_id, buffer_id, tex_w, tex_h, pixels);
        let Some(texture) = texture else { return; };

        let rect = self.to_ndc(x, y, dest_w, dest_h);
        unsafe {
            let enc = &frame.encoder;
            enc.setRenderPipelineState(&self.blit_pipeline);
            enc.setVertexBytes_length_atIndex(
                as_bytes(&rect), std::mem::size_of::<RectUniform>(), 0);
            enc.setFragmentTexture_atIndex(Some(&texture), 0);
            enc.drawPrimitives_vertexStart_vertexCount(MTLPrimitiveType::TriangleStrip, 0, 4);
        }
    }

    pub fn draw_shadow(&self, x: i32, y: i32, width: i32, height: i32, sigma: f32) {
        if width <= 0 || height <= 0 { return; }
        let frame_ref = self.frame.borrow();
        let Some(frame) = frame_ref.as_ref() else { return; };
        let rect = self.to_ndc(x, y, width, height);
        let color: [f32; 4] = [0.0, 0.0, 0.0, 0.5];
        unsafe {
            let enc = &frame.encoder;
            enc.setRenderPipelineState(&self.shadow_pipeline);
            enc.setVertexBytes_length_atIndex(
                as_bytes(&rect), std::mem::size_of::<RectUniform>(), 0);
            enc.setFragmentBytes_length_atIndex(
                slice_bytes(&color), std::mem::size_of_val(&color), 1);
            enc.setFragmentBytes_length_atIndex(
                as_bytes(&sigma), std::mem::size_of::<f32>(), 2);
            enc.drawPrimitives_vertexStart_vertexCount(MTLPrimitiveType::TriangleStrip, 0, 4);
        }
    }

    pub fn draw_border(&self, x: i32, y: i32, width: i32, height: i32, border_width: f32) {
        if width <= 0 || height <= 0 { return; }
        let frame_ref = self.frame.borrow();
        let Some(frame) = frame_ref.as_ref() else { return; };
        let rect = self.to_ndc(x, y, width, height);
        let color: [f32; 4] = [0.0, 0.6, 1.0, 1.0];
        let bw = border_width / width as f32;
        unsafe {
            let enc = &frame.encoder;
            enc.setRenderPipelineState(&self.border_pipeline);
            enc.setVertexBytes_length_atIndex(
                as_bytes(&rect), std::mem::size_of::<RectUniform>(), 0);
            enc.setFragmentBytes_length_atIndex(
                slice_bytes(&color), std::mem::size_of_val(&color), 1);
            enc.setFragmentBytes_length_atIndex(
                as_bytes(&bw), std::mem::size_of::<f32>(), 2);
            enc.drawPrimitives_vertexStart_vertexCount(MTLPrimitiveType::TriangleStrip, 0, 4);
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    /// Draw a solid-color filled rectangle.
    pub fn draw_rect(&self, x: i32, y: i32, w: i32, h: i32, color: [f32; 4]) {
        if w <= 0 || h <= 0 { return; }
        let frame_ref = self.frame.borrow();
        let Some(frame) = frame_ref.as_ref() else { return; };
        let rect = self.to_ndc(x, y, w, h);
        unsafe {
            let enc = &frame.encoder;
            enc.setRenderPipelineState(&self.solid_pipeline);
            enc.setVertexBytes_length_atIndex(
                as_bytes(&rect), std::mem::size_of::<RectUniform>(), 0);
            enc.setFragmentBytes_length_atIndex(
                slice_bytes(&color), std::mem::size_of_val(&color), 1);
            enc.drawPrimitives_vertexStart_vertexCount(MTLPrimitiveType::TriangleStrip, 0, 4);
        }
    }

    /// Draw a text label using CoreText, cached as a GPU texture.
    /// `color` is RGBA in [0,1]. `font_size` is in physical pixels.
    pub fn draw_text(&self, x: i32, y: i32, text: &str, color: [f32; 4], font_size: f64) {
        let cache_key = format!("{}|{:.2},{:.2},{:.2},{:.2}|{:.0}",
            text, color[0], color[1], color[2], color[3], font_size);

        // Check cache
        let (tex, tex_w, tex_h) = {
            let cache = self.text_cache.borrow();
            if let Some(entry) = cache.get(&cache_key) {
                (entry.0.clone(), entry.1, entry.2)
            } else {
                drop(cache);
                // Render via CoreText
                if let Some((new_tex, w, h)) = self.render_text_to_texture(text, font_size, color) {
                    self.text_cache.borrow_mut().insert(cache_key, (new_tex.clone(), w, h));
                    (new_tex, w, h)
                } else {
                    return;
                }
            }
        };

        // Draw the texture using the blit pipeline
        if tex_w == 0 || tex_h == 0 { return; }
        let frame_ref = self.frame.borrow();
        let Some(frame) = frame_ref.as_ref() else { return; };
        let rect = self.to_ndc(x, y, tex_w as i32, tex_h as i32);
        unsafe {
            let enc = &frame.encoder;
            enc.setRenderPipelineState(&self.blit_pipeline);
            enc.setVertexBytes_length_atIndex(
                as_bytes(&rect), std::mem::size_of::<RectUniform>(), 0);
            enc.setFragmentTexture_atIndex(Some(&tex), 0);
            enc.drawPrimitives_vertexStart_vertexCount(MTLPrimitiveType::TriangleStrip, 0, 4);
        }
    }

    /// Rasterize a text string via CoreText into an MTLTexture (BGRA8Unorm).
    fn render_text_to_texture(
        &self,
        text: &str,
        font_size: f64,
        color: [f32; 4],
    ) -> Option<(Retained<ProtocolObject<dyn MTLTexture>>, u32, u32)> {
        use core_graphics::{
            base::{kCGBitmapByteOrder32Host, kCGImageAlphaPremultipliedFirst, CGFloat},
            color_space::CGColorSpace,
            context::CGContext,
        };
        use core_foundation::{
            attributed_string::CFMutableAttributedString,
            base::TCFType,
            string::CFString,
        };
        use core_foundation_sys::base::CFRange;
        use core_text::{
            font::new_from_name,
            line::CTLine,
            string_attributes::kCTFontAttributeName,
        };

        // ── 1. Create font & attributed string ───────────────────────────────
        let font = new_from_name("SF Pro Text", font_size).unwrap_or_else(|_| {
            new_from_name(".AppleSystemUIFont", font_size)
                .unwrap_or_else(|_| new_from_name("Helvetica", font_size).unwrap())
        });

        let cf_text = CFString::new(text);
        let mut attr_str = CFMutableAttributedString::new();
        attr_str.replace_str(&cf_text, CFRange { location: 0, length: 0 });
        let range = CFRange { location: 0, length: cf_text.char_len() };
        attr_str.set_attribute::<core_text::font::CTFont>(range, unsafe { kCTFontAttributeName }, &font);

        let line = CTLine::new_with_attributed_string(attr_str.as_concrete_TypeRef());

        // ── 2. Measure bounds using a throwaway context ───────────────────────
        let cs = CGColorSpace::create_device_rgb();
        let bitmap_info = kCGBitmapByteOrder32Host | kCGImageAlphaPremultipliedFirst;
        let dummy_ctx = CGContext::create_bitmap_context(None, 1, 1, 8, 4, &cs, bitmap_info);
        let bounds = line.get_image_bounds(&dummy_ctx);

        let tex_w = (bounds.size.width.ceil() as u32 + 8).max(4);
        let tex_h = (bounds.size.height.ceil() as u32 + 8).max(4);
        let row_bytes = tex_w as usize * 4;

        // ── 3. Rasterize into a BGRA CPU buffer ───────────────────────────────
        let mut pixels = vec![0u8; tex_h as usize * row_bytes];
        let ctx = CGContext::create_bitmap_context(
            Some(pixels.as_mut_ptr() as *mut std::ffi::c_void),
            tex_w as usize,
            tex_h as usize,
            8,
            row_bytes,
            &cs,
            bitmap_info,
        );
        ctx.set_rgb_fill_color(
            color[0] as CGFloat,
            color[1] as CGFloat,
            color[2] as CGFloat,
            color[3] as CGFloat,
        );
        // CoreGraphics has bottom-left origin; nudge up so text is vertically centred
        let text_y = -bounds.origin.y as CGFloat + 4.0;
        ctx.set_text_position(4.0, text_y);
        line.draw(&ctx);

        // ── 4. Upload to MTLTexture ───────────────────────────────────────────
        let texture = unsafe {
            let desc = MTLTextureDescriptor::new();
            desc.setTextureType(MTLTextureType::MTLTextureType2D);
            desc.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
            desc.setWidth(tex_w as usize);
            desc.setHeight(tex_h as usize);
            desc.setUsage(MTLTextureUsage::ShaderRead);
            let tex = self.device.newTextureWithDescriptor(&desc)?;
            let region = MTLRegion {
                origin: MTLOrigin { x: 0, y: 0, z: 0 },
                size: MTLSize { width: tex_w as usize, height: tex_h as usize, depth: 1 },
            };
            tex.replaceRegion_mipmapLevel_withBytes_bytesPerRow(
                region, 0,
                NonNull::new_unchecked(pixels.as_mut_ptr() as *mut std::ffi::c_void),
                row_bytes,
            );
            tex
        };

        Some((texture, tex_w, tex_h))
    }

    fn to_ndc(&self, x: i32, y: i32, w: i32, h: i32) -> RectUniform {
        let fw = self.width  as f32;
        let fh = self.height as f32;
        RectUniform {
            x:  (2.0 * x as f32 / fw) - 1.0,
            y:  1.0 - (2.0 * (y + h) as f32 / fh),
            w:  2.0 * w as f32 / fw,
            h:  2.0 * h as f32 / fh,
        }
    }

    fn get_or_create_texture(&self,
                              surface_id: ObjectId, buffer_id: ObjectId,
                              tex_w: i32, tex_h: i32,
                              pixels: &[u8])
        -> Option<Retained<ProtocolObject<dyn MTLTexture>>>
    {
        let mut cache = self.texture_cache.borrow_mut();

        // Empty pixels = cache-hit path requested by caller.
        if pixels.is_empty() {
            return cache.get(&surface_id).and_then(|e| {
                if e.buffer_id == buffer_id { Some(e.texture.clone()) } else { None }
            });
        }

        if tex_w <= 0 || tex_h <= 0 || pixels.len() < (tex_w * tex_h * 4) as usize {
            cache.remove(&surface_id);
            return None;
        }

        let region = MTLRegion {
            origin: MTLOrigin { x: 0, y: 0, z: 0 },
            size:   MTLSize  { width: tex_w as usize, height: tex_h as usize, depth: 1 },
        };

        // Reuse existing texture if same size — avoids VRAM allocation every frame.
        if let Some(entry) = cache.get_mut(&surface_id) {
            if entry.tex_w == tex_w && entry.tex_h == tex_h {
                unsafe {
                    entry.texture.replaceRegion_mipmapLevel_withBytes_bytesPerRow(
                        region, 0, slice_bytes(pixels), (tex_w * 4) as usize);
                }
                entry.buffer_id = buffer_id;
                return Some(entry.texture.clone());
            }
        }

        // First frame or size changed: allocate a new MTLTexture.
        let texture = unsafe {
            let desc = MTLTextureDescriptor::new();
            desc.setTextureType(MTLTextureType::MTLTextureType2D);
            desc.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
            desc.setWidth(tex_w as usize);
            desc.setHeight(tex_h as usize);
            desc.setUsage(MTLTextureUsage::ShaderRead);
            let tex = self.device.newTextureWithDescriptor(&desc)?;
            tex.replaceRegion_mipmapLevel_withBytes_bytesPerRow(
                region, 0, slice_bytes(pixels), (tex_w * 4) as usize);
            tex
        };

        let cloned = texture.clone();
        cache.insert(surface_id, CachedTexture { texture, buffer_id, tex_w, tex_h });
        Some(cloned)
    }
}

unsafe fn make_pipeline(
    device:   &ProtocolObject<dyn MTLDevice>,
    vert:     &ProtocolObject<dyn MTLFunction>,
    frag:     &ProtocolObject<dyn MTLFunction>,
    blending: bool,
) -> Result<Retained<ProtocolObject<dyn MTLRenderPipelineState>>, String> {
    let desc = MTLRenderPipelineDescriptor::new();
    desc.setVertexFunction(Some(vert));
    desc.setFragmentFunction(Some(frag));

    let ca = unsafe { desc.colorAttachments().objectAtIndexedSubscript(0) };
    ca.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
    if blending {
        ca.setBlendingEnabled(true);
        ca.setSourceRGBBlendFactor(MTLBlendFactor::SourceAlpha);
        ca.setDestinationRGBBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
        ca.setSourceAlphaBlendFactor(MTLBlendFactor::One);
        ca.setDestinationAlphaBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
    }

    device.newRenderPipelineStateWithDescriptor_error(&desc)
        .map_err(|e: Retained<NSError>| format!("Pipeline error: {}", e.localizedDescription()))
}
