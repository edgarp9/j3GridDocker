use std::cell::RefCell;
use std::iter::once;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::COLORREF;
use windows_sys::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreatePen, CreateSolidBrush,
    DT_END_ELLIPSIS, DT_SINGLELINE, DT_VCENTER, DeleteDC, DeleteObject, DrawTextW, FillRect,
    HBRUSH, HDC, HGDIOBJ, HPEN, IntersectClipRect, PS_SOLID, Rectangle, RestoreDC, SRCCOPY, SaveDC,
    SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};

use super::ui::UiRect;

thread_local! {
    static PAINT_OBJECTS: RefCell<PaintObjectCache> = RefCell::new(PaintObjectCache::new());
}

struct PaintObjectCache {
    brushes: Vec<(COLORREF, HBRUSH)>,
    pens: Vec<(COLORREF, HPEN)>,
}

impl PaintObjectCache {
    fn new() -> Self {
        Self {
            brushes: Vec::new(),
            pens: Vec::new(),
        }
    }

    fn brush(&mut self, color: COLORREF) -> Option<HBRUSH> {
        if let Some((_, brush)) = self
            .brushes
            .iter()
            .find(|(cached_color, _)| *cached_color == color)
        {
            return Some(*brush);
        }

        let brush = unsafe { CreateSolidBrush(color) };
        if brush.is_null() {
            return None;
        }

        self.brushes.push((color, brush));
        Some(brush)
    }

    fn solid_pen(&mut self, color: COLORREF) -> Option<HPEN> {
        if let Some((_, pen)) = self
            .pens
            .iter()
            .find(|(cached_color, _)| *cached_color == color)
        {
            return Some(*pen);
        }

        let pen = unsafe { CreatePen(PS_SOLID, 1, color) };
        if pen.is_null() {
            return None;
        }

        self.pens.push((color, pen));
        Some(pen)
    }
}

impl Drop for PaintObjectCache {
    fn drop(&mut self) {
        unsafe {
            // Cached handles are only stored after successful Create* calls.
            for (_, pen) in self.pens.drain(..) {
                DeleteObject(pen as HGDIOBJ);
            }
            for (_, brush) in self.brushes.drain(..) {
                DeleteObject(brush as HGDIOBJ);
            }
        }
    }
}

fn with_cached_paint_objects(draw: impl FnOnce(&mut PaintObjectCache)) -> bool {
    PAINT_OBJECTS.with(|objects| {
        let Ok(mut objects) = objects.try_borrow_mut() else {
            return false;
        };
        draw(&mut objects);
        true
    })
}

pub(super) struct PaintBuffer {
    memory_dc: HDC,
    bitmap: HGDIOBJ,
    old_bitmap: HGDIOBJ,
    width: i32,
    height: i32,
}

impl PaintBuffer {
    pub(super) fn new() -> Self {
        Self {
            memory_dc: null_mut(),
            bitmap: null_mut(),
            old_bitmap: null_mut(),
            width: 0,
            height: 0,
        }
    }

    pub(super) fn paint(
        &mut self,
        hdc: HDC,
        client: UiRect,
        dirty: UiRect,
        mut paint: impl FnMut(HDC, UiRect),
    ) {
        let Some(dirty) = dirty.intersect(client) else {
            return;
        };

        if !self.ensure(hdc, client.width(), client.height()) {
            paint_clipped(hdc, dirty, |target| paint(target, dirty));
            return;
        }

        paint_clipped(self.memory_dc, dirty, |target| paint(target, dirty));

        unsafe {
            BitBlt(
                hdc,
                dirty.left,
                dirty.top,
                dirty.width(),
                dirty.height(),
                self.memory_dc,
                dirty.left,
                dirty.top,
                SRCCOPY,
            );
        }
    }

    fn ensure(&mut self, hdc: HDC, width: i32, height: i32) -> bool {
        if self.memory_dc.is_null() {
            self.memory_dc = unsafe { CreateCompatibleDC(hdc) };
            if self.memory_dc.is_null() {
                return false;
            }
        }

        if !self.bitmap.is_null() && self.width == width && self.height == height {
            return true;
        }

        self.replace_bitmap(hdc, width, height)
    }

    fn replace_bitmap(&mut self, hdc: HDC, width: i32, height: i32) -> bool {
        let bitmap = unsafe { CreateCompatibleBitmap(hdc, width, height) as HGDIOBJ };
        if bitmap.is_null() {
            return false;
        }

        let previous = unsafe { SelectObject(self.memory_dc, bitmap) };
        if previous.is_null() {
            unsafe {
                DeleteObject(bitmap);
            }
            return false;
        }

        if self.bitmap.is_null() {
            self.old_bitmap = previous;
        } else {
            unsafe {
                DeleteObject(previous);
            }
        }

        self.bitmap = bitmap;
        self.width = width;
        self.height = height;
        true
    }
}

fn paint_clipped(hdc: HDC, dirty: UiRect, mut paint: impl FnMut(HDC)) {
    let saved = unsafe { SaveDC(hdc) };
    if saved == 0 {
        paint(hdc);
        return;
    }

    unsafe {
        IntersectClipRect(hdc, dirty.left, dirty.top, dirty.right, dirty.bottom);
    }
    paint(hdc);
    unsafe {
        RestoreDC(hdc, saved);
    }
}

impl Default for PaintBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for PaintBuffer {
    fn drop(&mut self) {
        if self.memory_dc.is_null() {
            return;
        }

        unsafe {
            if !self.old_bitmap.is_null() {
                SelectObject(self.memory_dc, self.old_bitmap);
            }
            if !self.bitmap.is_null() {
                DeleteObject(self.bitmap);
            }
            DeleteDC(self.memory_dc);
        }
    }
}

pub(super) fn fill(hdc: HDC, rect: UiRect, color: COLORREF) {
    if with_cached_paint_objects(|objects| {
        let Some(brush) = objects.brush(color) else {
            return;
        };
        fill_with_brush(hdc, rect, brush);
    }) {
        return;
    }

    fill_uncached(hdc, rect, color);
}

fn fill_uncached(hdc: HDC, rect: UiRect, color: COLORREF) {
    let brush = unsafe { CreateSolidBrush(color) };
    if brush.is_null() {
        return;
    }
    fill_with_brush(hdc, rect, brush);
    unsafe {
        DeleteObject(brush as HGDIOBJ);
    }
}

fn fill_with_brush(hdc: HDC, rect: UiRect, brush: HBRUSH) {
    let raw = rect.to_rect();
    unsafe {
        // FillRect borrows the live brush handle only for this call.
        FillRect(hdc, &raw, brush);
    }
}

pub(super) fn draw_box(hdc: HDC, rect: UiRect, fill_color: COLORREF, border_color: COLORREF) {
    if with_cached_paint_objects(|objects| {
        let Some(brush) = objects.brush(fill_color) else {
            return;
        };
        let Some(pen) = objects.solid_pen(border_color) else {
            return;
        };
        draw_box_with_objects(hdc, rect, brush, pen);
    }) {
        return;
    }

    draw_box_uncached(hdc, rect, fill_color, border_color);
}

fn draw_box_uncached(hdc: HDC, rect: UiRect, fill_color: COLORREF, border_color: COLORREF) {
    let brush = unsafe { CreateSolidBrush(fill_color) };
    if brush.is_null() {
        return;
    }
    let pen = unsafe { CreatePen(PS_SOLID, 1, border_color) };
    if pen.is_null() {
        unsafe {
            DeleteObject(brush as HGDIOBJ);
        }
        return;
    }

    draw_box_with_objects(hdc, rect, brush, pen);
    unsafe {
        DeleteObject(pen as HGDIOBJ);
        DeleteObject(brush as HGDIOBJ);
    }
}

fn draw_box_with_objects(hdc: HDC, rect: UiRect, brush: HBRUSH, pen: HPEN) {
    unsafe {
        // The previous DC objects are restored before cached handles can be reused.
        let old_brush = SelectObject(hdc, brush as HGDIOBJ);
        let old_pen = SelectObject(hdc, pen as HGDIOBJ);
        Rectangle(hdc, rect.left, rect.top, rect.right, rect.bottom);
        SelectObject(hdc, old_pen);
        SelectObject(hdc, old_brush);
    }
}

pub(super) fn draw_text(hdc: HDC, rect: UiRect, text: &str, align: u32) {
    let wide = wide_null(text);
    draw_text_wide(hdc, rect, &wide, align);
}

pub(super) fn draw_text_wide(hdc: HDC, rect: UiRect, text: &[u16], align: u32) {
    if text.last().copied() != Some(0) {
        return;
    }

    let mut raw = rect.to_rect();
    unsafe {
        DrawTextW(
            hdc,
            text.as_ptr(),
            -1,
            &mut raw,
            DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | align,
        );
    }
}

pub(super) fn set_text(hdc: HDC, color: COLORREF) {
    unsafe {
        SetTextColor(hdc, color);
        SetBkMode(hdc, TRANSPARENT as i32);
    }
}

fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(once(0)).collect()
}
