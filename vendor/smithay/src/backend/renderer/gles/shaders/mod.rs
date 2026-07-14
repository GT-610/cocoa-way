mod implicit;

use std::fmt::Write;

pub use implicit::*;

// define constants
/// No alpha shader define
pub const NO_ALPHA: &str = "NO_ALPHA";
/// External texture shader define
pub const EXTERNAL: &str = "EXTERNAL";
/// Debug flags shader define
pub const DEBUG_FLAGS: &str = "DEBUG_FLAGS";

use super::*;

/// Compiles a shader variant.
///
/// # Safety
///
/// You must call this only when it is safe to compile shaders with GL.
pub unsafe fn compile_shader(
    gl: &ffi::Gles2,
    variant: ffi::types::GLuint,
    src: &str,
) -> Result<ffi::types::GLuint, GlesError> {
    let shader = gl.CreateShader(variant);
    if shader == 0 {
        return Err(GlesError::CreateShaderObject);
    }

    #[cfg(target_os = "macos")]
    let src_string = if src.contains("#version 100") || src.contains("#version 300 es") {
        let mut s = src.to_string();

        let is_vert = s.contains("attribute "); // Heuristic: only VS has attributes in this codebase.

        // 1. Version & Precision
        s = s
            .replace("#version 100", "#version 330 core")
            .replace("#version 300 es", "#version 330 core")
            .replace("precision mediump float;", "// precision mediump float;")
            .replace("precision highp float;", "// precision highp float;")
            .replace("precision lowp float;", "// precision lowp float;");

        // 2. Keywords
        if is_vert {
            // Vertex Shader
            // attribute -> in
            // varying -> out
            s = s.replace("attribute ", "in ").replace("varying ", "out ");
        } else {
            // Fragment Shader
            // varying -> in
            s = s.replace("varying ", "in ");

            // Handle Output
            // gl_FragColor -> fragColor
            if s.contains("gl_FragColor") {
                s = s
                    .replace("void main() {", "out vec4 fragColor;\nvoid main() {")
                    .replace("gl_FragColor", "fragColor");
            }
        }

        // 3. Functions
        // texture2D -> texture
        s = s.replace("texture2D", "texture");

        // 4. Uniforms
        // samplerExternalOES -> sampler2D (already patched in file, but careful)
        // If file still has samplerExternalOES, replace it.
        s = s.replace("samplerExternalOES", "sampler2D");

        s
    } else {
        src.to_string()
    };

    #[cfg(not(target_os = "macos"))]
    let src_string = src.to_string();

    let src_ptr = src_string.as_ptr();
    let src_len = src_string.len();

    gl.ShaderSource(
        shader,
        1,
        &src_ptr as *const *const u8 as *const *const ffi::types::GLchar,
        &(src_len as i32) as *const _,
    );
    gl.CompileShader(shader);

    let mut status = ffi::FALSE as i32;
    gl.GetShaderiv(shader, ffi::COMPILE_STATUS, &mut status as *mut _);
    if status == ffi::FALSE as i32 {
        let mut max_len = 0;
        gl.GetShaderiv(shader, ffi::INFO_LOG_LENGTH, &mut max_len as *mut _);

        let mut error = Vec::with_capacity(max_len as usize);
        let mut len = 0;
        gl.GetShaderInfoLog(
            shader,
            max_len as _,
            &mut len as *mut _,
            error.as_mut_ptr() as *mut _,
        );
        error.set_len(len as usize);

        let err_msg = std::str::from_utf8(&error).unwrap_or("<Error Message no utf8>");
        error!("[GL] Shader Error: {}", err_msg);
        // Print usage for debugging
        println!("[GL] Source: \n{}", src_string);

        gl.DeleteShader(shader);
        return Err(GlesError::ShaderCompileError);
    }

    Ok(shader)
}

/// Compiles and links a shader program.
///
/// # Safety
///
/// You must call this only when it is safe to compile and link shaders with GL.
pub unsafe fn link_program(
    gl: &ffi::Gles2,
    vert_src: &str,
    frag_src: &str,
) -> Result<ffi::types::GLuint, GlesError> {
    let vert = compile_shader(gl, ffi::VERTEX_SHADER, vert_src)?;
    let frag = compile_shader(gl, ffi::FRAGMENT_SHADER, frag_src)?;
    let program = gl.CreateProgram();
    gl.AttachShader(program, vert);
    gl.AttachShader(program, frag);
    gl.LinkProgram(program);
    gl.DetachShader(program, vert);
    gl.DetachShader(program, frag);
    gl.DeleteShader(vert);
    gl.DeleteShader(frag);

    let mut status = ffi::FALSE as i32;
    gl.GetProgramiv(program, ffi::LINK_STATUS, &mut status as *mut _);
    if status == ffi::FALSE as i32 {
        let mut max_len = 0;
        gl.GetProgramiv(program, ffi::INFO_LOG_LENGTH, &mut max_len as *mut _);

        let mut error = Vec::with_capacity(max_len as usize);
        let mut len = 0;
        gl.GetProgramInfoLog(
            program,
            max_len as _,
            &mut len as *mut _,
            error.as_mut_ptr() as *mut _,
        );
        error.set_len(len as usize);

        error!(
            "[GL] {}",
            std::str::from_utf8(&error).unwrap_or("<Error Message no utf8>")
        );

        gl.DeleteProgram(program);
        return Err(GlesError::ProgramLinkError);
    }

    Ok(program)
}

pub(super) unsafe fn texture_program(
    gl: &ffi::Gles2,
    src: &str,
    additional_uniforms: &[UniformName<'_>],
    destruction_callback_sender: Sender<CleanupResource>,
) -> Result<GlesTexProgram, GlesError> {
    let create_variant = |defines: &[&str]| -> Result<GlesTexProgramVariant, GlesError> {
        let shader = src.replace(
            "//_DEFINES_",
            &defines.iter().fold(String::new(), |mut shader, define| {
                let _ = writeln!(&mut shader, "#define {define}");
                shader
            }),
        );
        let debug_shader = src.replace(
            "//_DEFINES_",
            &defines
                .iter()
                .chain(&[shaders::DEBUG_FLAGS])
                .fold(String::new(), |mut shader, define| {
                    let _ = writeln!(shader, "#define {define}");
                    shader
                }),
        );

        let program = unsafe { link_program(gl, shaders::VERTEX_SHADER, &shader)? };
        let debug_program = unsafe { link_program(gl, shaders::VERTEX_SHADER, debug_shader.as_ref())? };

        let vert = c"vert";
        let vert_position = c"vert_position";
        let tex = c"tex";
        let matrix = c"matrix";
        let tex_matrix = c"tex_matrix";
        let alpha = c"alpha";
        let tint = c"tint";

        Ok(GlesTexProgramVariant {
            normal: GlesTexProgramInternal {
                program,
                uniform_tex: gl.GetUniformLocation(program, tex.as_ptr() as *const ffi::types::GLchar),
                uniform_matrix: gl.GetUniformLocation(program, matrix.as_ptr() as *const ffi::types::GLchar),
                uniform_tex_matrix: gl
                    .GetUniformLocation(program, tex_matrix.as_ptr() as *const ffi::types::GLchar),
                uniform_alpha: gl.GetUniformLocation(program, alpha.as_ptr() as *const ffi::types::GLchar),
                attrib_vert: gl.GetAttribLocation(program, vert.as_ptr() as *const ffi::types::GLchar),
                attrib_vert_position: gl
                    .GetAttribLocation(program, vert_position.as_ptr() as *const ffi::types::GLchar),
                additional_uniforms: additional_uniforms
                    .iter()
                    .map(|uniform| {
                        let name = CString::new(uniform.name.as_bytes()).expect("Interior null in name");
                        let location =
                            gl.GetUniformLocation(program, name.as_ptr() as *const ffi::types::GLchar);
                        (
                            uniform.name.clone().into_owned(),
                            UniformDesc {
                                location,
                                type_: uniform.type_,
                            },
                        )
                    })
                    .collect(),
            },
            debug: GlesTexProgramInternal {
                program: debug_program,
                uniform_tex: gl.GetUniformLocation(debug_program, tex.as_ptr() as *const ffi::types::GLchar),
                uniform_matrix: gl
                    .GetUniformLocation(debug_program, matrix.as_ptr() as *const ffi::types::GLchar),
                uniform_tex_matrix: gl
                    .GetUniformLocation(debug_program, tex_matrix.as_ptr() as *const ffi::types::GLchar),
                uniform_alpha: gl
                    .GetUniformLocation(debug_program, alpha.as_ptr() as *const ffi::types::GLchar),
                attrib_vert: gl.GetAttribLocation(debug_program, vert.as_ptr() as *const ffi::types::GLchar),
                attrib_vert_position: gl
                    .GetAttribLocation(debug_program, vert_position.as_ptr() as *const ffi::types::GLchar),
                additional_uniforms: additional_uniforms
                    .iter()
                    .map(|uniform| {
                        let name = CString::new(uniform.name.as_bytes()).expect("Interior null in name");
                        let location =
                            gl.GetUniformLocation(debug_program, name.as_ptr() as *const ffi::types::GLchar);
                        (
                            uniform.name.clone().into_owned(),
                            UniformDesc {
                                location,
                                type_: uniform.type_,
                            },
                        )
                    })
                    .collect(),
            },
            // debug flags
            uniform_tint: gl.GetUniformLocation(debug_program, tint.as_ptr() as *const ffi::types::GLchar),
        })
    };

    Ok(GlesTexProgram(Arc::new(GlesTexProgramInner {
        variants: [
            create_variant(&[])?,
            create_variant(&[shaders::NO_ALPHA])?,
            create_variant(&[shaders::EXTERNAL])?,
        ],
        destruction_callback_sender,
    })))
}

pub(super) unsafe fn solid_program(gl: &ffi::Gles2) -> Result<GlesSolidProgram, GlesError> {
    let program = link_program(gl, shaders::VERTEX_SHADER_SOLID, shaders::FRAGMENT_SHADER_SOLID)?;

    let matrix = c"matrix";
    let color = c"color";
    let vert = c"vert";
    let position = c"position";

    Ok(GlesSolidProgram {
        program,
        uniform_matrix: gl.GetUniformLocation(program, matrix.as_ptr() as *const ffi::types::GLchar),
        uniform_color: gl.GetUniformLocation(program, color.as_ptr() as *const ffi::types::GLchar),
        attrib_vert: gl.GetAttribLocation(program, vert.as_ptr() as *const ffi::types::GLchar),
        attrib_position: gl.GetAttribLocation(program, position.as_ptr() as *const ffi::types::GLchar),
    })
}
