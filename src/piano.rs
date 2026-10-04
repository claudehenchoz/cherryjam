//! On-screen piano: shows which computer key plays which note, lights up held notes, and can be
//! played with the mouse (click and drag for glissando).

use egui::{Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, Ui, Vec2};

use crate::keyboard::{KeyboardState, alt_label_for, label_for};
use crate::theme;

/// Number of semitones shown: two octaves plus the extra keys reachable from the top row (C..G).
pub const SPAN: u8 = 32;

const BLACK: [bool; 12] = [
    false, true, false, true, false, false, true, false, true, false, true, false,
];

fn is_black(n: u8) -> bool {
    BLACK[(n % 12) as usize]
}

#[derive(Default)]
pub struct Piano {
    mouse_note: Option<u8>,
}

struct KeyGeom {
    offset: u8,
    rect: Rect,
    black: bool,
}

fn layout(rect: Rect) -> Vec<KeyGeom> {
    let whites = (0..SPAN).filter(|&o| !is_black(o)).count() as f32;
    let ww = rect.width() / whites;
    let bw = ww * 0.62;
    let bh = rect.height() * 0.6;
    let mut keys = Vec::new();
    let mut wi = 0.0;
    for o in 0..SPAN {
        if is_black(o) {
            let x = rect.left() + wi * ww - bw / 2.0;
            keys.push(KeyGeom {
                offset: o,
                rect: Rect::from_min_size(Pos2::new(x, rect.top()), Vec2::new(bw, bh)),
                black: true,
            });
        } else {
            let x = rect.left() + wi * ww;
            keys.push(KeyGeom {
                offset: o,
                rect: Rect::from_min_size(Pos2::new(x, rect.top()), Vec2::new(ww, rect.height())),
                black: false,
            });
            wi += 1.0;
        }
    }
    keys
}

impl Piano {
    pub fn ui(&mut self, ui: &mut Ui, kb: &mut KeyboardState, height: f32) {
        let (rect, resp) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), height),
            Sense::click_and_drag(),
        );
        let keys = layout(rect);
        let base = kb.base_note;

        // Mouse playing: black keys sit on top, so test them first.
        let hovered = resp.interact_pointer_pos().and_then(|p| {
            keys.iter()
                .filter(|k| k.black)
                .chain(keys.iter().filter(|k| !k.black))
                .find(|k| k.rect.contains(p))
                .map(|k| base.saturating_add(k.offset))
        });
        let pressed = ui.input(|i| i.pointer.primary_down()) && resp.is_pointer_button_down_on();
        let want = if pressed { hovered } else { None };
        if want != self.mouse_note {
            if let Some(n) = self.mouse_note.take() {
                kb.note_off(n);
            }
            if let Some(n) = want {
                kb.note_on(n);
                self.mouse_note = Some(n);
            }
        }

        let painter = ui.painter_at(rect);
        let dark = ui.visuals().dark_mode;
        for black in [false, true] {
            for k in keys.iter().filter(|k| k.black == black) {
                let note = base.saturating_add(k.offset);
                let on = kb.is_note_on(note);
                let fill = match (black, on) {
                    (_, true) => theme::ACCENT,
                    (false, false) => {
                        if dark {
                            Color32::from_gray(232)
                        } else {
                            Color32::WHITE
                        }
                    }
                    (true, false) => Color32::from_gray(28),
                };
                let r = if black {
                    k.rect
                } else {
                    k.rect.shrink2(Vec2::new(1.0, 0.0))
                };
                painter.rect(
                    r,
                    CornerRadius {
                        nw: 0,
                        ne: 0,
                        sw: 4,
                        se: 4,
                    },
                    fill,
                    Stroke::new(1.0, Color32::from_gray(70)),
                    egui::StrokeKind::Inside,
                );

                let text_col = match (black, on) {
                    (_, true) => Color32::WHITE,
                    (false, false) => Color32::from_gray(90),
                    (true, false) => Color32::from_gray(190),
                };
                let size = (r.width() * 0.42).clamp(8.0, 15.0);
                if let Some(l) = label_for(k.offset) {
                    painter.text(
                        Pos2::new(r.center().x, r.bottom() - size * 0.9),
                        Align2::CENTER_CENTER,
                        l,
                        FontId::monospace(size),
                        text_col,
                    );
                }
                if let Some(l) = alt_label_for(k.offset) {
                    painter.text(
                        Pos2::new(r.center().x, r.bottom() - size * 2.1),
                        Align2::CENTER_CENTER,
                        l,
                        FontId::monospace(size * 0.85),
                        text_col.gamma_multiply(0.7),
                    );
                }
                if !black && note % 12 == 0 {
                    painter.text(
                        Pos2::new(r.center().x, r.top() + rect.height() * 0.68),
                        Align2::CENTER_CENTER,
                        format!("C{}", note as i32 / 12 - 1),
                        FontId::proportional(size * 0.8),
                        theme::ACCENT.gamma_multiply(if on { 0.0 } else { 0.9 }),
                    );
                }
            }
        }
    }

    /// Releases a note held by the mouse (e.g. when the window loses the pointer).
    pub fn release(&mut self, kb: &mut KeyboardState) {
        if let Some(n) = self.mouse_note.take() {
            kb.note_off(n);
        }
    }
}
