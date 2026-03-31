use smithay::reexports::wayland_server::backend::ObjectId;
use smithay::reexports::wayland_server::Resource;
use smithay::utils::{Logical, Point, Size};
use smithay::wayland::shell::xdg::ToplevelSurface;
#[derive(Debug)]
pub struct Tile {
    pub toplevel: ToplevelSurface,
    pub position: Point<i32, Logical>,
    pub size: Size<i32, Logical>,
    pub is_focused: bool,
    pub border_width: i32,
}
impl Tile {
    pub fn new(toplevel: ToplevelSurface, position: Point<i32, Logical>, size: Size<i32, Logical>) -> Self {
        Self {
            toplevel,
            position,
            size,
            is_focused: false,
            border_width: 2,
        }
    }
    pub fn surface_id(&self) -> ObjectId {
        self.toplevel.wl_surface().id()
    }
    pub fn bounds(&self) -> (Point<i32, Logical>, Size<i32, Logical>) {
        (self.position, self.size)
    }
    pub fn contains_point(&self, point: Point<f64, Logical>) -> bool {
        let x = point.x as i32;
        let y = point.y as i32;
        x >= self.position.x 
            && x < self.position.x + self.size.w
            && y >= self.position.y 
            && y < self.position.y + self.size.h
    }
    pub fn request_size(&self) {
        self.toplevel.with_pending_state(|state| {
            state.size = Some(self.size.into());
        });
        self.toplevel.send_configure();
    }
}
#[derive(Debug, Default)]
pub struct Layout {
    pub tiles: Vec<Tile>,
    pub focused_idx: Option<usize>,
    pub view_size: Size<i32, Logical>,
    pub gap: i32,
    pub padding: i32,
}
impl Layout {
    pub fn new(width: i32, height: i32) -> Self {
        Self {
            tiles: Vec::new(),
            focused_idx: None,
            view_size: Size::from((width, height)),
            gap: 0,
            padding: 0,
        }
    }
    pub fn set_view_size(&mut self, width: i32, height: i32) {
        self.view_size = Size::from((width, height));
        self.relayout();
    }
    pub fn add_tile(&mut self, toplevel: ToplevelSurface) {
        let position = self.calculate_new_tile_position();
        let size = self.calculate_tile_size(self.tiles.len() + 1);
        let tile = Tile::new(toplevel, position, size);
        self.tiles.push(tile);
        self.focused_idx = Some(self.tiles.len() - 1);
        self.relayout();
    }
    pub fn remove_tile(&mut self, surface_id: &ObjectId) {
        if let Some(idx) = self.tiles.iter().position(|t| &t.surface_id() == surface_id) {
            self.tiles.remove(idx);
            if let Some(focused) = self.focused_idx {
                if focused >= self.tiles.len() {
                    self.focused_idx = if self.tiles.is_empty() {
                        None
                    } else {
                        Some(self.tiles.len() - 1)
                    };
                }
            }
            self.relayout();
        }
    }
    fn calculate_tile_size(&self, count: usize) -> Size<i32, Logical> {
        if count == 0 {
            return self.view_size;
        }
        let available_w = self.view_size.w - self.padding * 2;
        let available_h = self.view_size.h - self.padding * 2;
        let tile_w = (available_w - self.gap * (count as i32 - 1)) / count as i32;
        let tile_h = available_h;
        Size::from((tile_w.max(100), tile_h.max(100)))
    }
    fn calculate_new_tile_position(&self) -> Point<i32, Logical> {
        Point::from((self.padding, self.padding))
    }
    pub fn relayout(&mut self) {
        let count = self.tiles.len();
        if count == 0 {
            return;
        }
        let tile_size = self.calculate_tile_size(count);
        let mut x = self.padding;
        for (i, tile) in self.tiles.iter_mut().enumerate() {
            tile.position = Point::from((x, self.padding));
            tile.size = tile_size;
            tile.is_focused = self.focused_idx == Some(i);
            tile.request_size();
            x += tile_size.w + self.gap;
        }
    }
    pub fn focus_at(&mut self, point: Point<f64, Logical>) -> Option<&Tile> {
        for (i, tile) in self.tiles.iter().enumerate() {
            if tile.contains_point(point) {
                self.focused_idx = Some(i);
                return Some(tile);
            }
        }
        None
    }
    pub fn focused_tile(&self) -> Option<&Tile> {
        self.focused_idx.and_then(|i| self.tiles.get(i))
    }
    pub fn tile_for_surface(&self, surface_id: &ObjectId) -> Option<&Tile> {
        self.tiles.iter().find(|t| &t.surface_id() == surface_id)
    }
    pub fn move_tile(&mut self, surface_id: &ObjectId, x: i32, y: i32) {
        if let Some(tile) = self.tiles.iter_mut().find(|t| &t.surface_id() == surface_id) {
            tile.position = Point::from((x, y));
        }
    }
    pub fn focus_next(&mut self) {
        if let Some(idx) = self.focused_idx {
            if !self.tiles.is_empty() {
                self.focused_idx = Some((idx + 1) % self.tiles.len());
            }
        } else if !self.tiles.is_empty() {
            self.focused_idx = Some(0);
        }
    }
    pub fn focus_prev(&mut self) {
        if let Some(idx) = self.focused_idx {
            if !self.tiles.is_empty() {
                self.focused_idx = Some(if idx == 0 { self.tiles.len() - 1 } else { idx - 1 });
            }
        } else if !self.tiles.is_empty() {
            self.focused_idx = Some(self.tiles.len() - 1);
        }
    }
}