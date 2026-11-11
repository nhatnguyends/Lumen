//! Shared content type scale for all Notes designs. Native fonts, no stretched
//! text bitmaps. The measurement DC/font and bounded wrap cache belong to State.
use super::*;
use std::collections::VecDeque;

#[derive(Clone, Copy)]
pub struct Metrics {
    pub body: f32,
    pub leading: f32,
    pub left: f32,
    pub top: f32,
    pub caption: f32,
    pub height: f32,
}
pub fn compact_card(size: [f32; 2]) -> bool {
    size[0] < 184. || size[1] < 112.
}
impl Metrics {
    pub fn text_rect(self, size: [f32; 2]) -> [f32; 4] {
        [
            self.left,
            self.top,
            size[0] - 17.,
            size[1] - if compact_card(size) { 10. } else { 34. },
        ]
    }
    pub fn for_board(design: &str, w: f32, h: f32) -> Self {
        let mut value = Self::for_theme(design);
        if w < 240. || h < 220. {
            value.left = 12.;
            value.top = 32.;
            value.leading = 20.;
        }
        value
    }

    pub fn for_theme(design: &str) -> Self {
        let mut value = Self::for_design(design == "notebook", design == "index");
        if design == "honey" {
            value.left = 20.;
            value.height = 176.;
            value.leading = 22.;
        }
        value
    }
    pub fn for_design(paper: bool, index: bool) -> Self {
        Self {
            body: 14.,
            leading: if paper { 24. } else { 20. },
            left: if paper {
                48.
            } else if index {
                56.
            } else {
                16.
            },
            top: 40.,
            caption: 11.,
            height: if paper {
                196.
            } else if index {
                128.
            } else {
                160.
            },
        }
    }
}
#[derive(PartialEq)]
struct Key {
    text: String,
    width: i32,
    dpi: u32,
    size: u32,
}
pub struct Typography {
    dc: HDC,
    font: HFONT,
    previous: HGDIOBJ,
    dpi: f32,
    size: f32,
    cache: VecDeque<(Key, Vec<String>)>,
}
impl Default for Typography {
    fn default() -> Self {
        Self {
            dc: null_mut(),
            font: null_mut(),
            previous: null_mut(),
            dpi: 0.,
            size: 0.,
            cache: VecDeque::new(),
        }
    }
}
impl Drop for Typography {
    fn drop(&mut self) {
        self.clear();
    }
}
impl Typography {
    pub fn clear(&mut self) {
        unsafe {
            if !self.dc.is_null() {
                if !self.previous.is_null() {
                    SelectObject(self.dc, self.previous);
                }
                if !self.font.is_null() {
                    DeleteObject(self.font);
                }
                DeleteDC(self.dc);
            }
            self.dc = null_mut();
            self.font = null_mut();
            self.previous = null_mut();
            self.cache.clear();
        }
    }
    unsafe fn prepare(&mut self, dpi: f32, size: f32) {
        if self.dpi == dpi && self.size == size && !self.dc.is_null() {
            return;
        }
        self.clear();
        self.dpi = dpi;
        self.size = size;
        self.dc = CreateCompatibleDC(null_mut());
        if self.dc.is_null() {
            return;
        }
        self.font = CreateFontW(
            -(size * dpi).round() as i32,
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            0,
            0,
            CLEARTYPE_QUALITY as u32,
            0,
            wide("Segoe UI Variable Text").as_ptr(),
        );
        if !self.font.is_null() {
            self.previous = SelectObject(self.dc, self.font);
        }
    }
    unsafe fn measure(&self, text: &str) -> f32 {
        if self.dc.is_null() || self.font.is_null() {
            return text.chars().count() as f32 * self.size * 0.6;
        }
        let text: Vec<u16> = text.encode_utf16().collect();
        let mut extent: SIZE = zeroed();
        if GetTextExtentPoint32W(self.dc, text.as_ptr(), text.len() as i32, &mut extent) == 0 {
            return text.len() as f32 * self.size * 0.6;
        }
        extent.cx as f32 / self.dpi
    }
    pub unsafe fn lines(
        &mut self,
        text: &str,
        width: f32,
        dpi: f32,
        size: f32,
        limit: usize,
    ) -> Vec<String> {
        if limit == 0 || width <= 0. {
            return vec![];
        }
        self.prepare(dpi, size);
        // Leave a small optical margin between GDI measurement and GDI+ glyphs.
        let width = (width - 3.).max(size);
        let key = Key {
            text: text.into(),
            width: (width * dpi).floor() as i32,
            dpi: dpi.to_bits(),
            size: size.to_bits(),
        };
        let all = if let Some((_, lines)) = self.cache.iter().find(|(k, _)| *k == key) {
            lines.clone()
        } else {
            let lines = wrap_measured(text, width, |s| self.measure(s));
            if self.cache.len() >= 32 {
                self.cache.pop_front();
            }
            self.cache.push_back((key, lines.clone()));
            lines
        };
        let more = all.len() > limit;
        let mut shown: Vec<_> = all.into_iter().take(limit).collect();
        if more {
            if let Some(last) = shown.last_mut() {
                while !last.is_empty() && self.measure(&format!("{last}…")) > width {
                    last.pop();
                }
                last.push('…');
            }
        }
        shown
    }
}
// Break long tokens without dropping their remaining text. Native measurement
// handles proportional letters and Vietnamese/Unicode instead of char counts.
fn wrap_measured(text: &str, width: f32, measure: impl Fn(&str) -> f32) -> Vec<String> {
    let mut result = Vec::new();
    for paragraph in text.lines() {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            let candidate = if line.is_empty() {
                word.into()
            } else {
                format!("{line} {word}")
            };
            if measure(&candidate) <= width {
                line = candidate;
                continue;
            }
            if !line.is_empty() {
                result.push(std::mem::take(&mut line));
            }
            for ch in word.chars() {
                let mut candidate = line.clone();
                candidate.push(ch);
                if !line.is_empty() && measure(&candidate) > width {
                    result.push(std::mem::take(&mut line));
                }
                line.push(ch);
            }
        }
        result.push(line);
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_font_measurement_wraps_and_marks_hidden_text_without_a_window() {
        unsafe {
            let mut typography = Typography::default();
            let lines=typography.lines("Tiếng Việt với khoảng cách rõ ràng. A longer paragraph for the compact note preview.",100.,1.,16.,2);
            assert_eq!(lines.len(), 2);
            assert!(lines.last().unwrap().ends_with('…'));
            assert!(typography.measure("WWWW") > typography.measure("iiii"));
            let wide = typography.lines("Tiếng Việt 😀", 1000., 1.5, 16., 5);
            assert_eq!(wide, vec!["Tiếng Việt 😀"]);
            typography.clear();
            assert!(typography.font.is_null() && typography.dc.is_null());
        }
    }
    #[test]
    fn proportional_wrap_preserves_long_unicode_tokens_and_paragraphs() {
        let text = "WWWWiiii\nTiếngViệtRấtDài\n\nfinal";
        let lines = wrap_measured(text, 6., |s| {
            s.chars().map(|c| if c == 'W' { 2. } else { 1. }).sum()
        });
        assert_eq!(lines.concat(), "WWWWiiiiTiếngViệtRấtDàifinal");
        assert!(lines.iter().any(String::is_empty));
        assert!(lines.iter().all(|s| s
            .chars()
            .map(|c| if c == 'W' { 2. } else { 1. })
            .sum::<f32>()
            <= 6.));
    }
}
