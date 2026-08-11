use crate::terminalstate::image::*;
use crate::terminalstate::{ImageAttachParams, PlacementInfo};
use crate::{StableRowIndex, TerminalState};
use ::image::{
    DynamicImage, GenericImage, GenericImageView, ImageBuffer, RgbImage, Rgba, RgbaImage,
};
use anyhow::Context;
use std::collections::{HashMap, HashSet};
use std::convert::TryFrom;
use std::io::Write;
use std::sync::Arc;
use std::time::Duration;
use wezterm_cell::color::ColorAttribute;
use wezterm_cell::image::{ImageCell, ImageCellAttachmentKind, ImageDataType, TextureCoordinate};
use wezterm_escape_parser::apc::{
    KittyFrameCompositionMode, KittyImage, KittyImageCompression, KittyImageData, KittyImageDelete,
    KittyImageFormat, KittyImageFrame, KittyImageFrameCompose, KittyImagePlacement,
    KittyImageTransmit, KittyImageVerbosity,
};
use wezterm_surface::change::ImageData;

#[derive(Debug, Default)]
pub struct KittyImageState {
    accumulator: Vec<KittyImage>,
    max_image_id: u32,
    number_to_id: HashMap<u32, u32>,
    id_to_data: HashMap<u32, Arc<ImageData>>,
    placements: HashMap<(u32, Option<u32>), PlacementInfo>,
    virtual_placements: Vec<KittyVirtualPlacement>,
    used_memory: usize,
    #[cfg(test)]
    placeholder_scan_count: usize,
    #[cfg(test)]
    placeholder_cell_scan_count: usize,
    #[cfg(test)]
    placeholder_attachment_update_count: usize,
}

#[derive(Debug, Clone)]
struct KittyVirtualPlacement {
    image_id: u32,
    placement_id: Option<u32>,
    columns: u32,
    rows: u32,
    image_width: u32,
    image_height: u32,
    data: Arc<ImageData>,
}

struct KittyPlaceholderGeometry {
    columns: u32,
    rows: u32,
    rendered_width: f64,
    rendered_height: f64,
    image_left: f64,
    image_top: f64,
}

struct PreparedKittyVirtualPlacement {
    image_id: u32,
    placement_id: Option<u32>,
    data: Arc<ImageData>,
    cell_width: usize,
    cell_height: usize,
    geometry: Option<KittyPlaceholderGeometry>,
    // A placeholder grid is normally small (Snacks uses at most 80x40).
    // Cache the derived cell slices so that a placement refresh does the
    // floating point geometry work once instead of once per cell.  Very large
    // protocol dimensions stay on the bounded, lazy path below rather than
    // allowing an untrusted c/r pair to allocate an enormous vector.
    tiles: Option<Vec<Option<ImageCell>>>,
}

impl PreparedKittyVirtualPlacement {
    fn new(placement: &KittyVirtualPlacement, cell_width: usize, cell_height: usize) -> Self {
        let geometry = (|| {
            if cell_width == 0
                || cell_height == 0
                || placement.image_width == 0
                || placement.image_height == 0
            {
                return None;
            }
            let cell_width_u32 = u32::try_from(cell_width).ok()?;
            let cell_height_u32 = u32::try_from(cell_height).ok()?;
            let natural_columns = placement.image_width.div_ceil(cell_width_u32);
            let natural_rows = placement.image_height.div_ceil(cell_height_u32);
            let (columns, rows) = match (placement.columns, placement.rows) {
                (0, 0) => (natural_columns, natural_rows),
                (columns, 0) => {
                    // If only one grid dimension is specified, derive the
                    // other from the source aspect ratio and the terminal
                    // cell aspect ratio.  Deriving the missing dimension
                    // independently from the raw pixels introduces visible
                    // letterboxing for non-square cells.
                    let grid_width = f64::from(columns) * cell_width as f64;
                    let grid_height = grid_width * f64::from(placement.image_height)
                        / f64::from(placement.image_width);
                    let rows = (grid_height / cell_height as f64).ceil();
                    if !rows.is_finite() || rows < 1.0 || rows > u32::MAX as f64 {
                        return None;
                    }
                    (columns, rows as u32)
                }
                (0, rows) => {
                    let grid_height = f64::from(rows) * cell_height as f64;
                    let grid_width = grid_height * f64::from(placement.image_width)
                        / f64::from(placement.image_height);
                    let columns = (grid_width / cell_width as f64).ceil();
                    if !columns.is_finite() || columns < 1.0 || columns > u32::MAX as f64 {
                        return None;
                    }
                    (columns as u32, rows)
                }
                (columns, rows) => (columns, rows),
            };
            if columns == 0 || rows == 0 {
                return None;
            }

            let grid_width = f64::from(columns) * cell_width as f64;
            let grid_height = f64::from(rows) * cell_height as f64;
            let scale = (grid_width / f64::from(placement.image_width))
                .min(grid_height / f64::from(placement.image_height));
            if !scale.is_finite() || scale <= 0.0 {
                return None;
            }
            let rendered_width = f64::from(placement.image_width) * scale;
            let rendered_height = f64::from(placement.image_height) * scale;
            Some(KittyPlaceholderGeometry {
                columns,
                rows,
                rendered_width,
                rendered_height,
                image_left: (grid_width - rendered_width) / 2.0,
                image_top: (grid_height - rendered_height) / 2.0,
            })
        })();

        let mut result = Self {
            image_id: placement.image_id,
            placement_id: placement.placement_id,
            data: Arc::clone(&placement.data),
            cell_width,
            cell_height,
            geometry,
            tiles: None,
        };

        if let Some(geometry) = result.geometry.as_ref() {
            const MAX_CACHED_PLACEHOLDER_TILES: usize = 65_536;
            let tile_count = geometry
                .columns
                .checked_mul(geometry.rows)
                .and_then(|count| usize::try_from(count).ok())
                .filter(|count| *count <= MAX_CACHED_PLACEHOLDER_TILES);
            if let Some(tile_count) = tile_count {
                let mut tiles = Vec::with_capacity(tile_count);
                for row in 0..geometry.rows {
                    for column in 0..geometry.columns {
                        tiles.push(result.compute_image_cell(row, column));
                    }
                }
                result.tiles = Some(tiles);
            }
        }

        result
    }

    fn image_cell(&self, row: u32, column: u32) -> Option<ImageCell> {
        if let Some(tiles) = &self.tiles {
            let geometry = self.geometry.as_ref()?;
            let index = row.checked_mul(geometry.columns)?.checked_add(column)?;
            return tiles.get(usize::try_from(index).ok()?)?.clone();
        }
        self.compute_image_cell(row, column)
    }

    fn image_cell_ref(&self, row: u32, column: u32) -> Option<&ImageCell> {
        let tiles = self.tiles.as_ref()?;
        let geometry = self.geometry.as_ref()?;
        let index = row.checked_mul(geometry.columns)?.checked_add(column)?;
        tiles.get(usize::try_from(index).ok()?)?.as_ref()
    }

    fn compute_image_cell(&self, row: u32, column: u32) -> Option<ImageCell> {
        let geometry = self.geometry.as_ref()?;
        if column >= geometry.columns || row >= geometry.rows {
            return None;
        }

        let cell_left = f64::from(column) * self.cell_width as f64;
        let cell_top = f64::from(row) * self.cell_height as f64;
        let cell_right = cell_left + self.cell_width as f64;
        let cell_bottom = cell_top + self.cell_height as f64;
        let left = cell_left.max(geometry.image_left);
        let top = cell_top.max(geometry.image_top);
        let right = cell_right.min(geometry.image_left + geometry.rendered_width);
        let bottom = cell_bottom.min(geometry.image_top + geometry.rendered_height);
        if left >= right || top >= bottom {
            return None;
        }

        let padding = |value: f64, maximum: usize| value.round().clamp(0.0, maximum as f64) as u16;
        Some(ImageCell::with_attachment_kind(
            TextureCoordinate::new_f32(
                ((left - geometry.image_left) / geometry.rendered_width) as f32,
                ((top - geometry.image_top) / geometry.rendered_height) as f32,
            ),
            TextureCoordinate::new_f32(
                ((right - geometry.image_left) / geometry.rendered_width) as f32,
                ((bottom - geometry.image_top) / geometry.rendered_height) as f32,
            ),
            Arc::clone(&self.data),
            -1,
            padding(left - cell_left, self.cell_width),
            padding(top - cell_top, self.cell_height),
            padding(cell_right - right, self.cell_width),
            padding(cell_bottom - bottom, self.cell_height),
            Some(self.image_id),
            self.placement_id,
            ImageCellAttachmentKind::KittyUnicodePlaceholder,
        ))
    }
}

struct KittyPlaceholderPlacementLookup {
    placements: Vec<PreparedKittyVirtualPlacement>,
    latest_by_image_id: HashMap<u32, usize>,
    exact_by_placement_id: HashMap<(u32, u32), usize>,
}

impl KittyPlaceholderPlacementLookup {
    fn new(placements: &[KittyVirtualPlacement], cell_width: usize, cell_height: usize) -> Self {
        let mut result = Self {
            placements: Vec::with_capacity(placements.len()),
            latest_by_image_id: HashMap::with_capacity(placements.len()),
            exact_by_placement_id: HashMap::with_capacity(placements.len()),
        };
        for placement in placements {
            let idx = result.placements.len();
            result.placements.push(PreparedKittyVirtualPlacement::new(
                placement,
                cell_width,
                cell_height,
            ));
            result.latest_by_image_id.insert(placement.image_id, idx);
            if let Some(placement_id) = placement.placement_id {
                result
                    .exact_by_placement_id
                    .insert((placement.image_id, placement_id), idx);
            }
        }
        result
    }

    fn resolve(
        &self,
        image_id: u32,
        requested_placement_id: u32,
    ) -> Option<&PreparedKittyVirtualPlacement> {
        let idx = if requested_placement_id == 0 {
            self.latest_by_image_id.get(&image_id)
        } else {
            self.exact_by_placement_id
                .get(&(image_id, requested_placement_id))
        }?;
        self.placements.get(*idx)
    }
}

const KITTY_UNICODE_PLACEHOLDER: char = '\u{10eeee}';

#[derive(Debug)]
struct KittyProtocolError {
    code: &'static str,
    printable_detail: String,
}

impl KittyProtocolError {
    fn new(code: &'static str, detail: impl std::fmt::Display) -> Self {
        Self {
            code,
            printable_detail: detail
                .to_string()
                .chars()
                .map(|ch| {
                    if ch.is_ascii_graphic() || ch == ' ' {
                        ch
                    } else {
                        '?'
                    }
                })
                .collect(),
        }
    }

    fn invalid(detail: impl std::fmt::Display) -> Self {
        Self::new("EINVAL", detail)
    }

    fn not_found(detail: impl std::fmt::Display) -> Self {
        Self::new("ENOENT", detail)
    }
}

impl std::fmt::Display for KittyProtocolError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}:{}", self.code, self.printable_detail)
    }
}

impl std::error::Error for KittyProtocolError {}

// The fixed table published by the Kitty graphics protocol.  Keep this table
// ordered: its zero-based index is the encoded row/column/high-byte value.
const KITTY_ROW_COLUMN_DIACRITICS: &[u32] = &[
    0x0305, 0x030D, 0x030E, 0x0310, 0x0312, 0x033D, 0x033E, 0x033F, 0x0346, 0x034A, 0x034B, 0x034C,
    0x0350, 0x0351, 0x0352, 0x0357, 0x035B, 0x0363, 0x0364, 0x0365, 0x0366, 0x0367, 0x0368, 0x0369,
    0x036A, 0x036B, 0x036C, 0x036D, 0x036E, 0x036F, 0x0483, 0x0484, 0x0485, 0x0486, 0x0487, 0x0592,
    0x0593, 0x0594, 0x0595, 0x0597, 0x0598, 0x0599, 0x059C, 0x059D, 0x059E, 0x059F, 0x05A0, 0x05A1,
    0x05A8, 0x05A9, 0x05AB, 0x05AC, 0x05AF, 0x05C4, 0x0610, 0x0611, 0x0612, 0x0613, 0x0614, 0x0615,
    0x0616, 0x0617, 0x0657, 0x0658, 0x0659, 0x065A, 0x065B, 0x065D, 0x065E, 0x06D6, 0x06D7, 0x06D8,
    0x06D9, 0x06DA, 0x06DB, 0x06DC, 0x06DF, 0x06E0, 0x06E1, 0x06E2, 0x06E4, 0x06E7, 0x06E8, 0x06EB,
    0x06EC, 0x0730, 0x0732, 0x0733, 0x0735, 0x0736, 0x073A, 0x073D, 0x073F, 0x0740, 0x0741, 0x0743,
    0x0745, 0x0747, 0x0749, 0x074A, 0x07EB, 0x07EC, 0x07ED, 0x07EE, 0x07EF, 0x07F0, 0x07F1, 0x07F3,
    0x0816, 0x0817, 0x0818, 0x0819, 0x081B, 0x081C, 0x081D, 0x081E, 0x081F, 0x0820, 0x0821, 0x0822,
    0x0823, 0x0825, 0x0826, 0x0827, 0x0829, 0x082A, 0x082B, 0x082C, 0x082D, 0x0951, 0x0953, 0x0954,
    0x0F82, 0x0F83, 0x0F86, 0x0F87, 0x135D, 0x135E, 0x135F, 0x17DD, 0x193A, 0x1A17, 0x1A75, 0x1A76,
    0x1A77, 0x1A78, 0x1A79, 0x1A7A, 0x1A7B, 0x1A7C, 0x1B6B, 0x1B6D, 0x1B6E, 0x1B6F, 0x1B70, 0x1B71,
    0x1B72, 0x1B73, 0x1CD0, 0x1CD1, 0x1CD2, 0x1CDA, 0x1CDB, 0x1CE0, 0x1DC0, 0x1DC1, 0x1DC3, 0x1DC4,
    0x1DC5, 0x1DC6, 0x1DC7, 0x1DC8, 0x1DC9, 0x1DCB, 0x1DCC, 0x1DD1, 0x1DD2, 0x1DD3, 0x1DD4, 0x1DD5,
    0x1DD6, 0x1DD7, 0x1DD8, 0x1DD9, 0x1DDA, 0x1DDB, 0x1DDC, 0x1DDD, 0x1DDE, 0x1DDF, 0x1DE0, 0x1DE1,
    0x1DE2, 0x1DE3, 0x1DE4, 0x1DE5, 0x1DE6, 0x1DFE, 0x20D0, 0x20D1, 0x20D4, 0x20D5, 0x20D6, 0x20D7,
    0x20DB, 0x20DC, 0x20E1, 0x20E7, 0x20E9, 0x20F0, 0x2CEF, 0x2CF0, 0x2CF1, 0x2DE0, 0x2DE1, 0x2DE2,
    0x2DE3, 0x2DE4, 0x2DE5, 0x2DE6, 0x2DE7, 0x2DE8, 0x2DE9, 0x2DEA, 0x2DEB, 0x2DEC, 0x2DED, 0x2DEE,
    0x2DEF, 0x2DF0, 0x2DF1, 0x2DF2, 0x2DF3, 0x2DF4, 0x2DF5, 0x2DF6, 0x2DF7, 0x2DF8, 0x2DF9, 0x2DFA,
    0x2DFB, 0x2DFC, 0x2DFD, 0x2DFE, 0x2DFF, 0xA66F, 0xA67C, 0xA67D, 0xA6F0, 0xA6F1, 0xA8E0, 0xA8E1,
    0xA8E2, 0xA8E3, 0xA8E4, 0xA8E5, 0xA8E6, 0xA8E7, 0xA8E8, 0xA8E9, 0xA8EA, 0xA8EB, 0xA8EC, 0xA8ED,
    0xA8EE, 0xA8EF, 0xA8F0, 0xA8F1, 0xAAB0, 0xAAB2, 0xAAB3, 0xAAB7, 0xAAB8, 0xAABE, 0xAABF, 0xAAC1,
    0xFE20, 0xFE21, 0xFE22, 0xFE23, 0xFE24, 0xFE25, 0xFE26, 0x10A0F, 0x10A38, 0x1D185, 0x1D186,
    0x1D187, 0x1D188, 0x1D189, 0x1D1AA, 0x1D1AB, 0x1D1AC, 0x1D1AD, 0x1D242, 0x1D243, 0x1D244,
];

impl KittyImageState {
    fn remove_data_for_id(&mut self, image_id: u32) {
        if let Some(data) = self.id_to_data.remove(&image_id) {
            self.used_memory = self.used_memory.saturating_sub(data.len());
        }
    }

    fn record_id_to_data(&mut self, image_id: u32, data: Arc<ImageData>) {
        if image_id != 0 {
            self.remove_data_for_id(image_id);
        }
        self.prune_unreferenced();
        self.used_memory += data.len();
        self.id_to_data.insert(image_id, data);
    }

    fn prune_unreferenced(&mut self) {
        let budget = 320 * 1024 * 1024; // FIXME: make this configurable
        if self.used_memory > budget {
            let referenced: HashSet<u32> = self
                .placements
                .keys()
                .map(|(k, _)| *k)
                .chain(self.virtual_placements.iter().map(|p| p.image_id))
                .collect();
            let target = self.used_memory - budget;
            let mut freed = 0;
            self.id_to_data.retain(|id, data| {
                if referenced.contains(id) || freed > target {
                    true
                } else {
                    freed += data.len();
                    false
                }
            });

            log::info!(
                "using {} RAM for images, pruned {}",
                self.used_memory,
                freed
            );
            self.used_memory = self.used_memory.saturating_sub(freed);
        }
    }
}

fn kitty_diacritic_index(ch: char) -> Option<u32> {
    KITTY_ROW_COLUMN_DIACRITICS
        .binary_search(&(ch as u32))
        .ok()
        .map(|idx| idx as u32)
}

fn kitty_color_id(color: ColorAttribute) -> u32 {
    match color {
        ColorAttribute::PaletteIndex(idx) => idx as u32,
        ColorAttribute::TrueColorWithPaletteFallback(rgb, _)
        | ColorAttribute::TrueColorWithDefaultFallback(rgb) => {
            let (r, g, b, _) = rgb.as_rgba_u8();
            u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b)
        }
        ColorAttribute::Default => 0,
    }
}

#[derive(Clone, Copy)]
struct PlaceholderRunCell {
    row: u32,
    column: u32,
    high: u32,
    foreground: ColorAttribute,
    underline: ColorAttribute,
}

fn decode_kitty_placeholder_cell(
    text: &str,
    foreground: ColorAttribute,
    underline: ColorAttribute,
    previous: Option<PlaceholderRunCell>,
) -> Option<PlaceholderRunCell> {
    let mut chars = text.chars();
    if chars.next() != Some(KITTY_UNICODE_PLACEHOLDER) {
        return None;
    }
    let mut marks = [None; 3];
    let mut mark_count = 0;
    for ch in chars.take(3) {
        marks[mark_count] = kitty_diacritic_index(ch);
        mark_count += 1;
    }
    let mut row = marks[0].unwrap_or(0);
    let mut column = marks[1].unwrap_or(0);
    let mut high = marks[2]
        .filter(|value| *value <= u8::MAX as u32)
        .unwrap_or(0);

    if let Some(left) =
        previous.filter(|left| left.foreground == foreground && left.underline == underline)
    {
        match mark_count {
            0 => {
                column = left.column.checked_add(1)?;
                row = left.row;
                high = left.high;
            }
            1 if marks[0].is_some() && row == left.row => {
                column = left.column.checked_add(1)?;
                high = left.high;
            }
            2 if marks[0].is_some() && marks[1].is_some() && row == left.row => {
                if left.column.checked_add(1) == Some(column) {
                    high = left.high;
                }
            }
            _ => {}
        }
    }

    Some(PlaceholderRunCell {
        row,
        column,
        high,
        foreground,
        underline,
    })
}

#[cfg(test)]
fn kitty_placeholder_image_cell(
    placement: &KittyVirtualPlacement,
    row: u32,
    column: u32,
    cell_width: usize,
    cell_height: usize,
) -> Option<ImageCell> {
    PreparedKittyVirtualPlacement::new(placement, cell_width, cell_height).image_cell(row, column)
}

impl TerminalState {
    pub(crate) fn has_kitty_virtual_placements(&self) -> bool {
        !self.kitty_img.virtual_placements.is_empty()
    }

    fn refresh_kitty_unicode_placeholder_line(
        line: &mut wezterm_surface::Line,
        placements: &KittyPlaceholderPlacementLookup,
        seqno: wezterm_surface::SequenceNo,
        force_scan: bool,
    ) -> (usize, usize) {
        if !force_scan && !line.has_kitty_unicode_placeholder() {
            return (0, 0);
        }

        let mut previous: Option<PlaceholderRunCell> = None;
        let mut has_placeholder = false;
        let mut attachment_updates = 0;
        let cells = line.cells_mut();
        let cells_scanned = cells.len();

        for cell in cells.iter_mut() {
            let foreground = cell.attrs().foreground();
            let underline = cell.attrs().underline_color();
            let run = decode_kitty_placeholder_cell(cell.str(), foreground, underline, previous);
            let changed = if let Some(run) = run {
                has_placeholder = true;
                previous = Some(run);
                let image_id = kitty_color_id(foreground) | (run.high << 24);
                let requested_placement = kitty_color_id(underline);
                match placements.resolve(image_id, requested_placement) {
                    Some(placement) => {
                        if let Some(image) = placement.image_cell_ref(run.row, run.column) {
                            cell.attrs_mut().replace_image_by_kind_ref(
                                ImageCellAttachmentKind::KittyUnicodePlaceholder,
                                Some(image),
                            )
                        } else {
                            let replacement = placement.image_cell(run.row, run.column);
                            cell.attrs_mut().replace_image_by_kind(
                                ImageCellAttachmentKind::KittyUnicodePlaceholder,
                                replacement,
                            )
                        }
                    }
                    None => cell.attrs_mut().replace_image_by_kind(
                        ImageCellAttachmentKind::KittyUnicodePlaceholder,
                        None,
                    ),
                }
            } else {
                previous = None;
                cell.attrs_mut()
                    .replace_image_by_kind(ImageCellAttachmentKind::KittyUnicodePlaceholder, None)
            };

            if changed {
                attachment_updates += 1;
            }
        }

        line.set_kitty_unicode_placeholder_flag(has_placeholder);
        if attachment_updates > 0 {
            line.update_last_change_seqno(seqno);
        }
        (cells_scanned, attachment_updates)
    }

    fn kitty_placeholder_refresh_geometry(&self) -> (usize, usize) {
        let columns = self.screen().physical_cols.max(1);
        let rows = self.screen().physical_rows.max(1);
        (self.pixel_width / columns, self.pixel_height / rows)
    }

    pub(crate) fn refresh_kitty_unicode_placeholders(&mut self) {
        let (cell_width, cell_height) = self.kitty_placeholder_refresh_geometry();
        let placements = KittyPlaceholderPlacementLookup::new(
            &self.kitty_img.virtual_placements,
            cell_width,
            cell_height,
        );
        let seqno = self.seqno;
        let mut _scan_count = 0;
        let mut _cell_scan_count = 0;
        let mut _attachment_update_count = 0;

        self.screen_mut().for_each_phys_line_mut(|_, line| {
            let (cells_scanned, attachments_updated) =
                Self::refresh_kitty_unicode_placeholder_line(line, &placements, seqno, false);
            if cells_scanned > 0 {
                _scan_count += 1;
                _cell_scan_count += cells_scanned;
                _attachment_update_count += attachments_updated;
            }
        });

        #[cfg(test)]
        {
            self.kitty_img.placeholder_scan_count += _scan_count;
            self.kitty_img.placeholder_cell_scan_count += _cell_scan_count;
            self.kitty_img.placeholder_attachment_update_count += _attachment_update_count;
        }
    }

    pub(crate) fn refresh_kitty_unicode_placeholders_in_stable_rows(
        &mut self,
        stable_rows: &HashSet<StableRowIndex>,
        force_scan: bool,
    ) {
        if stable_rows.is_empty() {
            return;
        }

        let (cell_width, cell_height) = self.kitty_placeholder_refresh_geometry();
        let placements = KittyPlaceholderPlacementLookup::new(
            &self.kitty_img.virtual_placements,
            cell_width,
            cell_height,
        );
        let seqno = self.seqno;
        let physical_rows: Vec<_> = stable_rows
            .iter()
            .filter_map(|stable| self.screen().stable_row_to_phys(*stable))
            .collect();
        let mut _scan_count = 0;
        let mut _cell_scan_count = 0;
        let mut _attachment_update_count = 0;

        for physical_row in physical_rows {
            let (cells_scanned, attachments_updated) = Self::refresh_kitty_unicode_placeholder_line(
                self.screen_mut().line_mut(physical_row),
                &placements,
                seqno,
                force_scan,
            );
            if cells_scanned > 0 {
                _scan_count += 1;
                _cell_scan_count += cells_scanned;
                _attachment_update_count += attachments_updated;
            }
        }

        #[cfg(test)]
        {
            self.kitty_img.placeholder_scan_count += _scan_count;
            self.kitty_img.placeholder_cell_scan_count += _cell_scan_count;
            self.kitty_img.placeholder_attachment_update_count += _attachment_update_count;
        }
    }

    #[cfg(test)]
    pub(crate) fn kitty_placeholder_refresh_stats(&self) -> (usize, usize, usize) {
        (
            self.kitty_img.placeholder_scan_count,
            self.kitty_img.placeholder_cell_scan_count,
            self.kitty_img.placeholder_attachment_update_count,
        )
    }

    fn kitty_img_place(
        &mut self,
        image_id: Option<u32>,
        image_number: Option<u32>,
        placement: KittyImagePlacement,
        verbosity: KittyImageVerbosity,
    ) -> anyhow::Result<()> {
        let image_id = match image_id {
            Some(id) => id,
            None => *self
                .kitty_img
                .number_to_id
                .get(&image_number.ok_or_else(|| {
                    KittyProtocolError::invalid("no image_id or image_number specified")
                })?)
                .ok_or_else(|| {
                    KittyProtocolError::not_found(format!(
                        "image_number has no matching image id {:?} in number_to_id",
                        image_number
                    ))
                })?,
        };

        log::trace!(
            "kitty_img_place image_id {:?} image_no {:?} placement {:?} verb {:?}",
            image_id,
            image_number,
            placement,
            verbosity
        );
        let placement_id = placement.placement_id.filter(|id| *id != 0);
        let img = Arc::clone(self.kitty_img.id_to_data.get(&image_id).ok_or_else(|| {
            KittyProtocolError::not_found(format!(
                "no matching image id {} in id_to_data for image_number {:?}",
                image_id, image_number
            ))
        })?);
        let (image_width, image_height) = img.data().dimensions()?;

        if placement.unicode_placeholder {
            if placement_id.is_some() {
                self.kitty_img.virtual_placements.retain(|candidate| {
                    candidate.image_id != image_id || candidate.placement_id != placement_id
                });
            }
            self.kitty_img
                .virtual_placements
                .push(KittyVirtualPlacement {
                    image_id,
                    placement_id,
                    columns: placement.columns.unwrap_or(0),
                    rows: placement.rows.unwrap_or(0),
                    image_width,
                    image_height,
                    data: img,
                });
            self.refresh_kitty_unicode_placeholders();
            return Ok(());
        }

        if image_id != 0 {
            self.kitty_remove_placement(image_id, placement.placement_id);
        }

        let info = self.assign_image_to_cells(ImageAttachParams {
            image_width,
            image_height,
            source_width: placement.w,
            source_height: placement.h,
            source_origin_x: placement.x.unwrap_or(0),
            source_origin_y: placement.y.unwrap_or(0),
            cell_padding_left: placement.x_offset.unwrap_or(0) as u16,
            cell_padding_top: placement.y_offset.unwrap_or(0) as u16,
            data: img,
            style: ImageAttachStyle::Kitty,
            z_index: placement.z_index.unwrap_or(0),
            columns: placement.columns.map(|x| x as usize),
            rows: placement.rows.map(|x| x as usize),
            image_id: Some(image_id),
            placement_id: placement.placement_id,
            do_not_move_cursor: placement.do_not_move_cursor,
        })?;

        self.kitty_img
            .placements
            .insert((image_id, placement.placement_id), info);
        log::trace!(
            "record placement for {} (image_number {:?}) {:?}",
            image_id,
            image_number,
            placement.placement_id
        );

        Ok(())
    }

    fn kitty_img_inner(&mut self, img: KittyImage) -> anyhow::Result<()> {
        match self
            .coalesce_kitty_accumulation(img)
            .context("coalesce_kitty_accumulation")?
        {
            KittyImage::TransmitData {
                transmit,
                verbosity,
            } => {
                self.kitty_img_transmit(transmit, verbosity)?;
                Ok(())
            }
            KittyImage::TransmitDataAndDisplay {
                transmit,
                placement,
                verbosity,
            } => {
                log::trace!("TransmitDataAndDisplay {:#?} {:#?}", transmit, placement);
                let image_number = transmit.image_number;
                let image_id = self.kitty_img_transmit(transmit, verbosity)?;
                self.kitty_img_place(Some(image_id), image_number, placement, verbosity)
            }
            _ => anyhow::bail!("impossible KittImage variant"),
        }
    }

    pub(crate) fn kitty_img(&mut self, img: KittyImage) -> anyhow::Result<()> {
        log::trace!("{:?}", img);
        if !self.config.enable_kitty_graphics() {
            return Ok(());
        }
        let verbosity = img.verbosity();
        let (mut response_image_id, response_image_number, response_placement_id) = match &img {
            KittyImage::Invalid {
                image_id,
                image_number,
                placement_id,
                ..
            } => (*image_id, *image_number, *placement_id),
            KittyImage::TransmitData { transmit, .. }
            | KittyImage::TransmitFrame { transmit, .. }
            | KittyImage::Query { transmit, .. } => {
                (transmit.image_id, transmit.image_number, None)
            }
            KittyImage::TransmitDataAndDisplay {
                transmit,
                placement,
                ..
            } => (
                transmit.image_id,
                transmit.image_number,
                placement.placement_id,
            ),
            KittyImage::Display {
                image_id,
                image_number,
                placement,
                ..
            } => (*image_id, *image_number, placement.placement_id),
            KittyImage::Delete {
                what:
                    KittyImageDelete::ByImageId {
                        image_id,
                        placement_id,
                        ..
                    },
                ..
            } => (Some(*image_id), None, *placement_id),
            KittyImage::Delete {
                what:
                    KittyImageDelete::ByImageNumber {
                        image_number,
                        placement_id,
                        ..
                    },
                ..
            } => (None, Some(*image_number), *placement_id),
            KittyImage::Delete { .. } | KittyImage::ComposeFrame { .. } => (None, None, None),
        };
        let mut respond = true;
        let result: anyhow::Result<()> = match img {
            KittyImage::Invalid {
                error,
                respond: can_respond,
                ..
            } => {
                respond = can_respond;
                Err(KittyProtocolError::invalid(error).into())
            }
            KittyImage::Query { transmit, .. } => transmit
                .data
                .load_data()
                .context("validating query image data")
                .map(|_| ()),
            KittyImage::TransmitData {
                transmit,
                verbosity,
            } => {
                let more_data_follows = transmit.more_data_follows;
                let img = KittyImage::TransmitData {
                    transmit,
                    verbosity,
                };
                if more_data_follows {
                    self.kitty_img.accumulator.push(img);
                    respond = false;
                    Ok(())
                } else {
                    self.kitty_img_inner(img)
                }
            }
            KittyImage::TransmitDataAndDisplay {
                transmit,
                placement,
                verbosity,
            } => {
                let more_data_follows = transmit.more_data_follows;
                let img = KittyImage::TransmitDataAndDisplay {
                    transmit,
                    placement,
                    verbosity,
                };
                if more_data_follows {
                    self.kitty_img.accumulator.push(img);
                    respond = false;
                    Ok(())
                } else {
                    self.kitty_img_inner(img)
                }
            }
            KittyImage::Display {
                image_id,
                image_number,
                placement,
                verbosity,
            } => self.kitty_img_place(image_id, image_number, placement, verbosity),
            KittyImage::Delete { what, verbosity: _ } => {
                // A delete always terminates a partial transfer, even when the
                // selector does not ultimately match an image.
                self.kitty_img.accumulator.clear();
                match what {
                    KittyImageDelete::ByImageId {
                        image_id,
                        placement_id,
                        delete,
                    } => {
                        let placement_id = placement_id.filter(|id| *id != 0);
                        let real_exists = self.kitty_img.placements.keys().any(|(id, pid)| {
                            *id == image_id && (placement_id.is_none() || *pid == placement_id)
                        });
                        let virtual_exists =
                            self.kitty_img.virtual_placements.iter().any(|candidate| {
                                candidate.image_id == image_id
                                    && (placement_id.is_none()
                                        || candidate.placement_id == placement_id)
                            });
                        if (placement_id.is_some() && !real_exists && !virtual_exists)
                            || (placement_id.is_none()
                                && !real_exists
                                && !virtual_exists
                                && !self.kitty_img.id_to_data.contains_key(&image_id))
                        {
                            Err(anyhow::anyhow!(
                                "ENOENT:no image or placement for id {image_id}"
                            ))
                        } else {
                            self.kitty_remove_placement(image_id, placement_id);
                            self.kitty_img.virtual_placements.retain(|candidate| {
                                candidate.image_id != image_id
                                    || (placement_id.is_some()
                                        && candidate.placement_id != placement_id)
                            });
                            if delete && !self.kitty_image_is_referenced(image_id) {
                                self.kitty_img.remove_data_for_id(image_id);
                            }
                            self.refresh_kitty_unicode_placeholders();
                            Ok(())
                        }
                    }
                    KittyImageDelete::ByImageNumber {
                        image_number,
                        placement_id,
                        delete,
                    } => match self.kitty_img.number_to_id.get(&image_number).copied() {
                        Some(image_id) => {
                            let placement_id = placement_id.filter(|id| *id != 0);
                            if placement_id.is_some()
                                && !self
                                    .kitty_img
                                    .placements
                                    .keys()
                                    .any(|(id, pid)| *id == image_id && *pid == placement_id)
                                && !self.kitty_img.virtual_placements.iter().any(|candidate| {
                                    candidate.image_id == image_id
                                        && candidate.placement_id == placement_id
                                })
                            {
                                Err(anyhow::anyhow!("ENOENT:no matching placement"))
                            } else {
                                self.kitty_remove_placement(image_id, placement_id);
                                self.kitty_img.virtual_placements.retain(|candidate| {
                                    candidate.image_id != image_id
                                        || (placement_id.is_some()
                                            && candidate.placement_id != placement_id)
                                });
                                if delete && !self.kitty_image_is_referenced(image_id) {
                                    self.kitty_img.remove_data_for_id(image_id);
                                }
                                self.refresh_kitty_unicode_placeholders();
                                Ok(())
                            }
                        }
                        None => Err(anyhow::anyhow!(
                            "ENOENT:no image for image number {image_number}"
                        )),
                    },
                    KittyImageDelete::ByImageIdRange {
                        first_image_id,
                        last_image_id,
                        delete,
                    } => {
                        if first_image_id > last_image_id {
                            Err(anyhow::anyhow!("EINVAL:image id range is reversed"))
                        } else {
                            let ids: HashSet<u32> = self
                                .kitty_img
                                .id_to_data
                                .keys()
                                .chain(self.kitty_img.placements.keys().map(|(id, _)| id))
                                .chain(
                                    self.kitty_img
                                        .virtual_placements
                                        .iter()
                                        .map(|p| &p.image_id),
                                )
                                .copied()
                                .filter(|id| *id >= first_image_id && *id <= last_image_id)
                                .collect();
                            if ids.is_empty() {
                                Err(anyhow::anyhow!("ENOENT:no images in requested range"))
                            } else {
                                for image_id in ids {
                                    self.kitty_remove_placement(image_id, None);
                                    self.kitty_img
                                        .virtual_placements
                                        .retain(|p| p.image_id != image_id);
                                    if delete {
                                        self.kitty_img.remove_data_for_id(image_id);
                                    }
                                }
                                self.refresh_kitty_unicode_placeholders();
                                Ok(())
                            }
                        }
                    }
                    KittyImageDelete::All { delete } => {
                        self.kitty_remove_all_placements(delete);
                        Ok(())
                    }
                    other => Err(anyhow::anyhow!(
                        "EINVAL:delete selector is not implemented: {other:?}"
                    )),
                }
            }
            KittyImage::TransmitFrame {
                transmit,
                frame,
                verbosity,
            } => self.kitty_frame_transmit(transmit, frame, verbosity),
            KittyImage::ComposeFrame { frame, verbosity } => {
                self.kitty_frame_compose(frame, verbosity)
            }
        };

        if response_image_id.is_none() {
            if let Some(image_number) = response_image_number {
                response_image_id = self.kitty_img.number_to_id.get(&image_number).copied();
            }
        }
        if respond {
            let (success, message) = match &result {
                Ok(()) => (true, "OK".to_string()),
                Err(err) => {
                    let (code, detail) =
                        if let Some(protocol) = err.downcast_ref::<KittyProtocolError>() {
                            (protocol.code, protocol.printable_detail.clone())
                        } else {
                            let detail = format!("{err:#}")
                                .chars()
                                .map(|ch| {
                                    if ch.is_ascii_graphic() || ch == ' ' {
                                        ch
                                    } else {
                                        '?'
                                    }
                                })
                                .collect::<String>();
                            let code = if detail.starts_with("ENOENT:") {
                                "ENOENT"
                            } else {
                                "EINVAL"
                            };
                            let detail = detail
                                .strip_prefix("ENOENT:")
                                .or_else(|| detail.strip_prefix("EINVAL:"))
                                .unwrap_or(&detail)
                                .to_string();
                            (code, detail)
                        };
                    (false, format!("{code}:{detail}"))
                }
            };
            self.kitty_send_response(
                verbosity,
                success,
                response_image_id,
                response_image_number,
                response_placement_id,
                message,
            )?;
        }

        result
    }

    fn kitty_remove_placement_from_model(
        &mut self,
        image_id: u32,
        placement_id: Option<u32>,
        info: PlacementInfo,
    ) {
        let seqno = self.seqno;
        let screen = self.screen_mut();
        let range =
            screen.stable_range(&(info.first_row..info.first_row + info.rows as StableRowIndex));
        for idx in range {
            let line = screen.line_mut(idx);
            for c in line.cells_mut() {
                c.attrs_mut()
                    .detach_image_with_placement(image_id, placement_id);
            }
            line.update_last_change_seqno(seqno);
        }
    }

    fn kitty_remove_placement(&mut self, image_id: u32, placement_id: Option<u32>) {
        if placement_id.is_some() {
            if let Some(info) = self.kitty_img.placements.remove(&(image_id, placement_id)) {
                log::trace!("removed placement {} {:?}", image_id, placement_id);
                self.kitty_remove_placement_from_model(image_id, placement_id, info);
            }
        } else {
            let mut to_clear = vec![];
            for (id, p) in self.kitty_img.placements.keys() {
                if *id == image_id {
                    to_clear.push(*p);
                }
            }
            for p in to_clear.into_iter() {
                if let Some(info) = self.kitty_img.placements.remove(&(image_id, p)) {
                    self.kitty_remove_placement_from_model(image_id, p, info);
                }
            }
        }

        log::trace!(
            "after remove: there are {} placements, {} images, {} memory",
            self.kitty_img.placements.len(),
            self.kitty_img.id_to_data.len(),
            self.kitty_img.used_memory,
        );
    }

    fn kitty_image_is_referenced(&self, image_id: u32) -> bool {
        self.kitty_img
            .placements
            .keys()
            .any(|(id, _)| *id == image_id)
            || self
                .kitty_img
                .virtual_placements
                .iter()
                .any(|placement| placement.image_id == image_id)
    }

    fn kitty_retire_image_id(&mut self, image_id: u32) {
        self.kitty_remove_placement(image_id, None);
        self.kitty_img
            .virtual_placements
            .retain(|placement| placement.image_id != image_id);
        self.kitty_img.remove_data_for_id(image_id);
        self.kitty_img
            .number_to_id
            .retain(|_, mapped_id| *mapped_id != image_id);
        self.refresh_kitty_unicode_placeholders();
    }

    pub(crate) fn kitty_remove_all_placements(&mut self, delete: bool) {
        for ((image_id, p), info) in std::mem::take(&mut self.kitty_img.placements).into_iter() {
            self.kitty_remove_placement_from_model(image_id, p, info);
        }
        if delete {
            let virtual_ids: HashSet<u32> = self
                .kitty_img
                .virtual_placements
                .iter()
                .map(|placement| placement.image_id)
                .collect();
            self.kitty_img
                .id_to_data
                .retain(|image_id, _| virtual_ids.contains(image_id));
            self.kitty_img.used_memory = self
                .kitty_img
                .id_to_data
                .values()
                .map(|data| data.len())
                .sum();
            self.kitty_img
                .number_to_id
                .retain(|_, image_id| virtual_ids.contains(image_id));
        }
    }

    fn kitty_send_response(
        &mut self,
        verbosity: KittyImageVerbosity,
        success: bool,
        image_id: Option<u32>,
        image_no: Option<u32>,
        placement_id: Option<u32>,
        message: String,
    ) -> anyhow::Result<()> {
        match verbosity {
            KittyImageVerbosity::Verbose => {}
            KittyImageVerbosity::OnlyErrors => {
                if success {
                    return Ok(());
                }
            }
            KittyImageVerbosity::Quiet => {
                return Ok(());
            }
        }

        log::trace!("Query Response: {}", message);

        let placement = placement_id
            .map(|id| format!(",p={id}"))
            .unwrap_or_default();
        match (image_id, image_no) {
            (Some(id), Some(no)) => {
                write!(
                    self.writer,
                    "\x1b_GI={},i={}{};{}\x1b\\",
                    no, id, placement, message
                )
                .context("writing Kitty graphics response")?;
            }
            (Some(id), None) => {
                write!(self.writer, "\x1b_Gi={}{};{}\x1b\\", id, placement, message)
                    .context("writing Kitty graphics response")?;
            }
            (None, Some(no)) => {
                write!(self.writer, "\x1b_GI={}{};{}\x1b\\", no, placement, message)
                    .context("writing Kitty graphics response")?;
            }
            (None, None) => {
                write!(
                    self.writer,
                    "\x1b_G{};{}\x1b\\",
                    placement.trim_start_matches(','),
                    message
                )
                .context("writing Kitty graphics response")?;
            }
        }
        self.writer
            .flush()
            .context("flushing Kitty graphics response")?;
        self.writer
            .get_mut()
            .flush_and_wait()
            .context("waiting for Kitty graphics response writer")?;
        Ok(())
    }

    fn kitty_frame_compose(
        &mut self,
        frame: KittyImageFrameCompose,
        _verbosity: KittyImageVerbosity,
    ) -> anyhow::Result<()> {
        let image_id = match frame.image_number {
            Some(no) => match self.kitty_img.number_to_id.get(&no) {
                Some(id) => *id,
                None => {
                    anyhow::bail!("ENOENT:no such image_number {}", no);
                }
            },
            None => frame
                .image_id
                .ok_or_else(|| anyhow::anyhow!("ENOENT:no image_id"))?,
        };

        let src_frame = frame
            .source_frame
            .ok_or_else(|| anyhow::anyhow!("ENOENT:missing source frame"))?
            as usize;
        let target_frame = frame
            .target_frame
            .ok_or_else(|| anyhow::anyhow!("ENOENT:missing target frame"))?
            as usize;

        let img = self
            .kitty_img
            .id_to_data
            .get(&image_id)
            .ok_or_else(|| anyhow::anyhow!("invalid image id {}", image_id))?;

        let mut img = img.data();
        match &mut *img {
            ImageDataType::EncodedLease(_) | ImageDataType::EncodedFile(_) => {
                anyhow::bail!("invalid image type")
            }
            ImageDataType::Rgba8 {
                width,
                height,
                data,
                hash,
            } => {
                anyhow::ensure!(
                    src_frame == target_frame && src_frame == 1,
                    "src_frame={} target_frame={} but there is only a single frame",
                    src_frame,
                    target_frame
                );

                let src = clip_view(
                    *width,
                    *height,
                    data.as_mut_slice(),
                    frame.src_x,
                    frame.src_y,
                    frame.w,
                    frame.h,
                )?;

                let mut dest: ImageBuffer<Rgba<u8>, &mut [u8]> =
                    ImageBuffer::from_raw(*width, *height, data.as_mut_slice())
                        .ok_or_else(|| anyhow::anyhow!("ill formed image"))?;

                blit(
                    &mut dest,
                    &src,
                    frame.x.unwrap_or(0),
                    frame.y.unwrap_or(0),
                    frame.composition_mode,
                )?;

                drop(dest);

                *hash = ImageDataType::hash_bytes(data);
            }
            ImageDataType::AnimRgba8 {
                width,
                height,
                frames,
                hashes,
                ..
            } => {
                anyhow::ensure!(
                    src_frame > 0 && src_frame <= frames.len(),
                    "src_frame {} is out of range",
                    src_frame
                );
                anyhow::ensure!(
                    target_frame > 0 && target_frame <= frames.len(),
                    "target_frame {} is out of range",
                    target_frame
                );

                let src = clip_view(
                    *width,
                    *height,
                    frames[src_frame - 1].as_mut_slice(),
                    frame.src_x,
                    frame.src_y,
                    frame.w,
                    frame.h,
                )?;

                let mut dest: ImageBuffer<Rgba<u8>, &mut [u8]> =
                    ImageBuffer::from_raw(*width, *height, frames[target_frame - 1].as_mut_slice())
                        .ok_or_else(|| anyhow::anyhow!("ill formed image"))?;

                blit(
                    &mut dest,
                    &src,
                    frame.x.unwrap_or(0),
                    frame.y.unwrap_or(0),
                    frame.composition_mode,
                )?;

                drop(dest);
                hashes[target_frame - 1] = ImageDataType::hash_bytes(&frames[target_frame - 1]);
            }
        }

        Ok(())
    }

    fn kitty_frame_transmit(
        &mut self,
        mut transmit: KittyImageTransmit,
        frame: KittyImageFrame,
        _verbosity: KittyImageVerbosity,
    ) -> anyhow::Result<()> {
        if let Some(no) = transmit.image_number.take() {
            match self.kitty_img.number_to_id.get(&no) {
                Some(id) => {
                    transmit.image_id.replace(*id);
                }
                None => {
                    transmit.image_number.replace(no);
                }
            }
        }

        let (image_id, image_number, img) = self.kitty_img_transmit_inner(transmit)?;

        let img = match img.decode() {
            ImageDataType::Rgba8 {
                data,
                width,
                height,
                ..
            } => RgbaImage::from_vec(width, height, data)
                .ok_or_else(|| anyhow::anyhow!("data isn't rgba8"))?,
            wat => anyhow::bail!("data isn't rgba8 {:?}", wat),
        };

        let background_pixel = frame.background_pixel.unwrap_or(0);
        let background_pixel = Rgba([
            ((background_pixel >> 24) & 0xff) as u8,
            ((background_pixel >> 16) & 0xff) as u8,
            ((background_pixel >> 8) & 0xff) as u8,
            (background_pixel & 0xff) as u8,
        ]);

        let anim = match self.kitty_img.id_to_data.get(&image_id) {
            Some(anim) => anim,
            None => {
                anyhow::bail!(
                    "ENOENT:no matching image id {} in id_to_data for image_number {:?}",
                    image_id,
                    image_number
                )
            }
        };

        let mut anim = anim.data();
        let x = frame.x.unwrap_or(0);
        let y = frame.y.unwrap_or(0);
        let frame_gap = Duration::from_millis(match frame.duration_ms {
            None | Some(0) => 40,
            Some(n) => n.into(),
        });

        match &mut *anim {
            ImageDataType::EncodedLease(_) | ImageDataType::EncodedFile(_) => {
                anyhow::bail!("Expected decoded image for image id {}", image_id)
            }
            ImageDataType::Rgba8 {
                data,
                width,
                height,
                hash,
            } => {
                let base_frame = match frame.base_frame {
                    Some(1) => Some(1),
                    None => None,
                    Some(n) => anyhow::bail!(
                        "attempted to copy frame {} but there is only a single frame",
                        n
                    ),
                };

                match frame.frame_number {
                    Some(1) => {
                        // Edit in place
                        let len = data.len();
                        let mut anim_img: ImageBuffer<Rgba<u8>, &mut [u8]> =
                            ImageBuffer::from_raw(*width, *height, data.as_mut_slice())
                                .ok_or_else(|| {
                                    anyhow::anyhow!(
                                        "ImageBuffer::from_raw failed for single \
                                         frame of {}x{} ({} bytes)",
                                        width,
                                        height,
                                        len
                                    )
                                })?;

                        blit(&mut anim_img, &img, x, y, frame.composition_mode)?;

                        drop(anim_img);
                        *hash = ImageDataType::hash_bytes(data);
                    }
                    Some(2) | None => {
                        // Create a second frame

                        let mut new_frame = if base_frame.is_some() {
                            RgbaImage::from_vec(*width, *height, data.clone()).unwrap()
                        } else {
                            RgbaImage::from_pixel(*width, *height, background_pixel)
                        };

                        blit(&mut new_frame, &img, x, y, frame.composition_mode)?;

                        let new_frame_data = new_frame.into_vec();
                        let new_frame_hash = ImageDataType::hash_bytes(&new_frame_data);

                        let frames = vec![std::mem::take(data), new_frame_data];
                        let durations = vec![Duration::from_millis(0), frame_gap];
                        let hashes = vec![*hash, new_frame_hash];

                        *anim = ImageDataType::AnimRgba8 {
                            width: *width,
                            height: *height,
                            frames,
                            durations,
                            hashes,
                        };
                    }
                    Some(n) => anyhow::bail!(
                        "attempted to edit frame {} but there is only a single frame",
                        n
                    ),
                }
            }
            ImageDataType::AnimRgba8 {
                width,
                height,
                frames,
                durations,
                hashes,
            } => {
                let frame_no = frame.frame_number.unwrap_or(frames.len() as u32 + 1);
                if frame_no == frames.len() as u32 + 1 {
                    // Append a new frame

                    let mut new_frame = match frame.base_frame {
                        None => RgbaImage::from_pixel(*width, *height, background_pixel),
                        Some(n) => {
                            let n = n as usize;
                            anyhow::ensure!(
                                n > 0 && n <= frames.len(),
                                "attempted to copy frame {} which is outside range 1-{}",
                                n,
                                frames.len()
                            );
                            RgbaImage::from_vec(*width, *height, frames[n - 1].clone()).unwrap()
                        }
                    };

                    blit(&mut new_frame, &img, x, y, frame.composition_mode)?;

                    let new_frame_data = new_frame.into_vec();
                    let new_frame_hash = ImageDataType::hash_bytes(&new_frame_data);

                    frames.push(new_frame_data);
                    hashes.push(new_frame_hash);
                    durations.push(frame_gap);
                } else {
                    anyhow::ensure!(
                        frame_no > 0 && frame_no <= frames.len() as u32,
                        "attempted to edit frame {} which is outside range 1-{}",
                        frame_no,
                        frames.len()
                    );

                    let frame_no = frame_no as usize;

                    let len = frames[frame_no - 1].len();
                    let mut anim_img: ImageBuffer<Rgba<u8>, &mut [u8]> =
                        ImageBuffer::from_raw(*width, *height, frames[frame_no - 1].as_mut_slice())
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "ImageBuffer::from_raw failed for single \
                                         frame of {}x{} ({} bytes)",
                                    width,
                                    height,
                                    len
                                )
                            })?;

                    blit(&mut anim_img, &img, x, y, frame.composition_mode)?;

                    drop(anim_img);
                    hashes[frame_no - 1] = ImageDataType::hash_bytes(&frames[frame_no - 1]);
                }
            }
        }

        Ok(())
    }

    fn kitty_img_transmit_inner(
        &mut self,
        transmit: KittyImageTransmit,
    ) -> anyhow::Result<(u32, Option<u32>, ImageDataType)> {
        log::trace!("transmit {:?}", transmit);
        let (id, no) = match (transmit.image_id, transmit.image_number) {
            (Some(_), Some(_)) => {
                // TODO: send an EINVAL error back here
                anyhow::bail!("cannot use both i= and I= in the same request");
            }
            (None, None) => {
                // Assume image id 0
                (0, None)
            }
            (Some(id), None) => (id, None),
            (None, Some(no)) => {
                let id = self
                    .kitty_img
                    .max_image_id
                    .checked_add(1)
                    .ok_or_else(|| anyhow::anyhow!("EINVAL:image id space exhausted"))?;
                (id, Some(no))
            }
        };

        let data = transmit
            .data
            .load_data()
            .context("data should have been materialized in coalesce_kitty_accumulation")?;

        let data = match transmit.compression {
            KittyImageCompression::None => data,
            KittyImageCompression::Deflate => {
                miniz_oxide::inflate::decompress_to_vec_zlib(&data)
                    .map_err(|e| anyhow::anyhow!("decompressing data: {:?}", e))?
            }
        };

        let img = match transmit.format {
            None | Some(KittyImageFormat::Rgba) | Some(KittyImageFormat::Rgb) => {
                let (width, height) = match (transmit.width, transmit.height) {
                    (Some(w), Some(h)) => (w, h),
                    _ => {
                        anyhow::bail!("missing width/height info for kitty img");
                    }
                };

                check_image_dimensions(width, height)?;

                let data = match transmit.format {
                    Some(KittyImageFormat::Rgb) => {
                        let img = DynamicImage::ImageRgb8(
                            RgbImage::from_vec(width, height, data)
                                .ok_or_else(|| anyhow::anyhow!("failed to decode image"))?,
                        );
                        let img = img.into_rgba8();
                        img.into_vec()
                    }
                    _ => data,
                };

                anyhow::ensure!(
                    width * height * 4 == data.len() as u32,
                    "transmit data len is {} but it doesn't match width*height*4 {}x{}x4 = {}",
                    data.len(),
                    width,
                    height,
                    width * height * 4
                );

                ImageDataType::new_single_frame(width, height, data)
            }
            Some(KittyImageFormat::Png) => {
                let info = dimensions(&data)?;
                check_image_dimensions(info.width, info.height)?;
                let decoded = image::load_from_memory(&data).context("decode png")?;
                let (width, height) = decoded.dimensions();
                let data = decoded.into_rgba8().into_vec();
                ImageDataType::new_single_frame(width, height, data)
            }
        };

        Ok((id, no, img))
    }

    fn kitty_img_transmit(
        &mut self,
        transmit: KittyImageTransmit,
        _verbosity: KittyImageVerbosity,
    ) -> anyhow::Result<u32> {
        if let Some(image_id) = transmit.image_id {
            self.kitty_retire_image_id(image_id);
        }
        let (image_id, image_number, img) = self.kitty_img_transmit_inner(transmit)?;
        self.kitty_img.max_image_id = self.kitty_img.max_image_id.max(image_id);

        let img = self
            .raw_image_to_image_data(img)
            .context("storing image data")?;
        self.kitty_img.record_id_to_data(image_id, img);
        if let Some(image_number) = image_number {
            self.kitty_img.number_to_id.insert(image_number, image_id);
        }

        Ok(image_id)
    }

    fn coalesce_kitty_accumulation(&mut self, img: KittyImage) -> anyhow::Result<KittyImage> {
        if self.kitty_img.accumulator.is_empty() {
            Ok(img)
        } else {
            let mut data = vec![];
            let mut trans;
            let place;
            let final_verbosity = img.verbosity();

            self.kitty_img.accumulator.push(img);

            let mut empty_data = KittyImageData::Direct(String::new());
            match self.kitty_img.accumulator.remove(0) {
                KittyImage::TransmitData { transmit, .. } => {
                    trans = transmit;
                    place = None;
                    std::mem::swap(&mut empty_data, &mut trans.data);
                }
                KittyImage::TransmitDataAndDisplay {
                    transmit,
                    placement,
                    ..
                } => {
                    place = Some(placement);
                    trans = transmit;
                    std::mem::swap(&mut empty_data, &mut trans.data);
                }
                _ => unreachable!(),
            }
            data.push(empty_data);

            for item in self.kitty_img.accumulator.drain(..) {
                match item {
                    KittyImage::TransmitData { transmit, .. }
                    | KittyImage::TransmitDataAndDisplay { transmit, .. } => {
                        data.push(transmit.data);
                    }
                    _ => unreachable!(),
                }
            }

            let mut b64_decoded = vec![];
            for mut data in data.into_iter() {
                match &mut data {
                    KittyImageData::DirectBin(b) => {
                        b64_decoded.append(b);
                    }
                    KittyImageData::Direct(b) => {
                        if !b.is_empty() {
                            b64_decoded.append(&mut data.load_data()?);
                        }
                    }
                    data => {
                        anyhow::bail!("expected data chunks to be Direct data, found {:#?}", data)
                    }
                }
            }

            trans.data = KittyImageData::DirectBin(b64_decoded);

            if let Some(placement) = place {
                Ok(KittyImage::TransmitDataAndDisplay {
                    transmit: trans,
                    placement,
                    verbosity: final_verbosity,
                })
            } else {
                Ok(KittyImage::TransmitData {
                    transmit: trans,
                    verbosity: final_verbosity,
                })
            }
        }
    }
}

/// Make a copy of the source region.
/// Ideally we wouldn't need this, but Rust's mutability rules
/// make it very awkward to mutably reference a frame while
/// an immutable reference exists to a separate frame.
fn clip_view(
    width: u32,
    height: u32,
    data: &mut [u8],
    src_x: Option<u32>,
    src_y: Option<u32>,
    view_width: Option<u32>,
    view_height: Option<u32>,
) -> anyhow::Result<RgbaImage> {
    let src = ImageBuffer::from_raw(width, height, data)
        .ok_or_else(|| anyhow::anyhow!("ill formed image"))?;

    let src_x = src_x.unwrap_or(0);
    let src_y = src_y.unwrap_or(0);

    let view_width = view_width.unwrap_or(width);
    let view_height = view_height.unwrap_or(height);

    let (view_width, view_height) =
        image::imageops::overlay_bounds((width, height), (view_width, view_height), src_x, src_y);

    let view = src.view(src_x, src_y, view_width, view_height);

    let mut tmp = RgbaImage::new(view_width, view_height);
    tmp.copy_from(&*view, 0, 0).context("copy source image")?;
    Ok(tmp)
}

fn blit<D, S, P>(
    dest: &mut D,
    src: &S,
    x: u32,
    y: u32,
    mode: KittyFrameCompositionMode,
) -> anyhow::Result<()>
where
    D: GenericImage<Pixel = P>,
    S: GenericImageView<Pixel = P>,
{
    match mode {
        KittyFrameCompositionMode::Overwrite => {
            ::image::imageops::replace(dest, src, x.into(), y.into());
        }
        KittyFrameCompositionMode::AlphaBlending => {
            ::image::imageops::overlay(dest, src, x.into(), y.into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod unicode_placeholder_test {
    use super::*;

    fn mark(index: usize) -> char {
        char::from_u32(KITTY_ROW_COLUMN_DIACRITICS[index]).unwrap()
    }

    fn placeholder(indices: &[usize]) -> String {
        let mut text = KITTY_UNICODE_PLACEHOLDER.to_string();
        text.extend(indices.iter().map(|index| mark(*index)));
        text
    }

    #[test]
    fn diacritic_bounds_and_inheritance_rules() {
        assert_eq!(
            kitty_color_id(ColorAttribute::TrueColorWithDefaultFallback(
                (42u8, 43u8, 44u8).into(),
            )),
            0x2a2b2c
        );
        assert_eq!(kitty_diacritic_index(mark(0)), Some(0));
        assert_eq!(kitty_diacritic_index(mark(255)), Some(255));
        assert_eq!(kitty_diacritic_index(mark(256)), Some(256));
        assert_eq!(kitty_diacritic_index(mark(296)), Some(296));
        assert_eq!(kitty_diacritic_index('\u{0300}'), None);

        let foreground = ColorAttribute::PaletteIndex(7);
        let underline = ColorAttribute::PaletteIndex(9);
        let first =
            decode_kitty_placeholder_cell(&placeholder(&[1, 2, 3]), foreground, underline, None)
                .unwrap();
        assert_eq!((first.row, first.column, first.high), (1, 2, 3));

        for text in [placeholder(&[]), placeholder(&[1]), placeholder(&[1, 3])] {
            let inherited =
                decode_kitty_placeholder_cell(&text, foreground, underline, Some(first)).unwrap();
            assert_eq!((inherited.row, inherited.column, inherited.high), (1, 3, 3));
        }

        let high_out_of_range = decode_kitty_placeholder_cell(
            &placeholder(&[0, 0, 256, 1]),
            foreground,
            underline,
            None,
        )
        .unwrap();
        assert_eq!(high_out_of_range.high, 0);

        let color_break = decode_kitty_placeholder_cell(
            &placeholder(&[1]),
            ColorAttribute::PaletteIndex(8),
            underline,
            Some(first),
        )
        .unwrap();
        assert_eq!(
            (color_break.row, color_break.column, color_break.high),
            (1, 0, 0)
        );
    }

    #[test]
    fn contain_geometry_centers_letterbox_and_clips_cells() {
        let data = Arc::new(ImageData::with_data(ImageDataType::new_single_frame(
            20,
            10,
            vec![0; 20 * 10 * 4],
        )));
        let placement = KittyVirtualPlacement {
            image_id: 1,
            placement_id: Some(2),
            columns: 2,
            rows: 2,
            image_width: 20,
            image_height: 10,
            data,
        };
        let top = kitty_placeholder_image_cell(&placement, 0, 0, 10, 10).unwrap();
        assert_eq!(top.padding(), (0, 5, 0, 0));
        assert_eq!(top.top_left().y.into_inner(), 0.0);
        assert_eq!(top.bottom_right().y.into_inner(), 0.5);

        let bottom = kitty_placeholder_image_cell(&placement, 1, 1, 10, 10).unwrap();
        assert_eq!(bottom.padding(), (0, 0, 0, 5));
        assert_eq!(bottom.top_left().y.into_inner(), 0.5);
        assert_eq!(bottom.bottom_right().y.into_inner(), 1.0);
        assert!(kitty_placeholder_image_cell(&placement, 2, 0, 10, 10).is_none());
    }

    #[test]
    fn one_explicit_grid_dimension_preserves_image_aspect_ratio() {
        let data = Arc::new(ImageData::with_data(ImageDataType::new_single_frame(
            100,
            100,
            vec![0; 100 * 100 * 4],
        )));
        let placement = KittyVirtualPlacement {
            image_id: 1,
            placement_id: None,
            columns: 20,
            rows: 0,
            image_width: 100,
            image_height: 100,
            data,
        };
        let prepared = PreparedKittyVirtualPlacement::new(&placement, 10, 20);
        let geometry = prepared.geometry.as_ref().unwrap();
        assert_eq!((geometry.columns, geometry.rows), (20, 10));
        assert!(prepared.tiles.is_some());
    }
}
