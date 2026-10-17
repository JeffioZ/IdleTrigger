//! Shared layout tokens of the UI kit: logical-pixel spacings, control
//! heights, and slot math. Every value scales through `scale`/`scale_pub`
//! at use sites, so the tokens stay resolution-independent. Windows own
//! their own client dimensions; everything below is the shared grammar the
//! panel, settings, and manager surfaces are converging on.

/// Standard content inset from a window's client edge.
pub(crate) const PAD: i32 = 12;
/// Default gap between unrelated neighbors.
pub(crate) const GAP: i32 = 8;
/// Vertical space between section cards.
pub(crate) const CARD_GAP: i32 = 10;
/// Card face inset: rows sit this far inside the card's rounded border.
pub(crate) const CARD_PAD_X: i32 = 12;
pub(crate) const CARD_PAD_Y: i32 = 8;
/// Gap between stacked rows inside a card.
pub(crate) const LABEL_GAP: i32 = 6;
/// Gap between the outside title row and the top of its card.
pub(crate) const TITLE_GAP: i32 = 4;
/// Section title row height (outside, above its card).
pub(crate) const SECTION_H: i32 = 18;
/// Status/subtitle line height.
pub(crate) const SUBTITLE_H: i32 = 20;
/// Standard interactive row height (buttons, switch rows).
pub(crate) const BUTTON_H: i32 = 36;
/// Preset chips are secondary shortcuts: shorter than real buttons so they
/// read as a quiet toolbar strip under their section row.
pub(crate) const CHIP_H: i32 = 28;
/// Hit column reserved at a row's right edge for a pill switch.
pub(crate) const SWITCH_HIT_W: i32 = 60;
/// Header action links wrap their text exactly — hit and display boxes
/// coincide, so centering the box centers the text.
pub(crate) const LINK_BOX_H: i32 = 20;

/// Equal-width slot `index` of `count` in a GAP-separated row, spreading
/// the remainder pixels over the leftmost slots so every equal-split row
/// ends flush with the row's right padding.
pub(crate) fn row_slot(row_width: i32, count: i32, index: i32) -> (i32, i32) {
    if count <= 0 {
        return (0, 0);
    }
    let gaps = (count - 1) * GAP;
    let base = (row_width - gaps) / count;
    let remainder = (row_width - gaps) % count;
    let width = base + i32::from(index < remainder);
    let x = index * (base + GAP) + index.min(remainder);
    (x, width)
}
