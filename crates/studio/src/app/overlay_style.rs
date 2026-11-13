// LCK_V201_OVERLAY_STYLE
// Display-only visual configuration for editor overlays.
// No game records or PS1 export data are modified.

use std::{
    fs,
    path::PathBuf,
    sync::{Mutex, OnceLock},
};

use eframe::egui;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Style {
    pub grid_color: [u8; 3],
    pub grid_transparency: f32,
    pub collision_color: [u8; 3],
    pub collision_transparency: f32,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            grid_color: [89, 166, 196],
            grid_transparency: 0.78,
            collision_color: [238, 164, 96],
            collision_transparency: 0.76,
        }
    }
}

impl Style {
    fn sanitize(&mut self) {
        let defaults = Self::default();

        if !self.grid_transparency.is_finite() {
            self.grid_transparency = defaults.grid_transparency;
        }

        if !self.collision_transparency.is_finite() {
            self.collision_transparency =
                defaults.collision_transparency;
        }

        self.grid_transparency =
            self.grid_transparency.clamp(0.0, 1.0);

        self.collision_transparency =
            self.collision_transparency.clamp(0.0, 1.0);
    }

    pub(crate) fn grid_opacity(&self) -> f32 {
        (1.0 - self.grid_transparency).clamp(0.0, 1.0)
    }

    pub(crate) fn collision_opacity(&self) -> f32 {
        (1.0 - self.collision_transparency).clamp(0.0, 1.0)
    }

    pub(crate) fn grid_major_color(&self) -> [u8; 3] {
        self.grid_color.map(|c| {
            (c as f32 * 1.22 + 9.0).min(255.0) as u8
        })
    }

    pub(crate) fn added_collision_color(&self) -> [u8; 3] {
        self.collision_color.map(|c| {
            (c as f32 * 0.72 + 60.0).min(255.0) as u8
        })
    }

    pub(crate) fn alpha_for_id(&self, id: u32) -> Option<f32> {
        let grid = self.grid_opacity();
        let collision = self.collision_opacity();

        match id {
            // Existing native-grid ribbon geometry.
            // V2.0.3: legacy raster ribbons hidden.
            // Continuous grid lines are painted after raster.
            x if x == u32::MAX - 1 => Some(0.0),

            // New native-grid translucent surface panels.
            x if x == u32::MAX - 5 => Some(grid),

            // Existing native collision outlines.
            x if x == u32::MAX - 3 => Some(
                if collision <= 0.0 {
                    0.0
                } else {
                    (0.58 + 0.42 * collision).min(1.0)
                }
            ),

            // New translucent collision faces.
            x if x == u32::MAX - 4 => Some(collision),

            // Selected boxes remain clearly highlighted.
            x if x == u32::MAX - 7 => Some(1.0),
            x if x == u32::MAX - 8 => Some(0.32),

            _ => None,
        }
    }
}

pub(crate) fn is_overlay_id(id: u32) -> bool {
    matches!(
        id,
        x if x == u32::MAX - 1
            || x == u32::MAX - 3
            || x == u32::MAX - 4
            || x == u32::MAX - 5
            || x == u32::MAX - 7
            || x == u32::MAX - 8
    )
}

fn settings_path() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()?
        .parent()
        .map(|dir| dir.join("LCK_VIEW_STYLE.json"))
}

fn load() -> Style {
    let Some(path) = settings_path() else {
        return Style::default();
    };

    let mut style = fs::read_to_string(path)
        .ok()
        .and_then(|data| serde_json::from_str::<Style>(&data).ok())
        .unwrap_or_default();

    style.sanitize();
    style
}

fn storage() -> &'static Mutex<Style> {
    static CURRENT: OnceLock<Mutex<Style>> = OnceLock::new();

    CURRENT.get_or_init(|| Mutex::new(load()))
}

pub(crate) fn snapshot() -> Style {
    storage()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

fn save(style: &Style) -> Result<(), String> {
    let path = settings_path()
        .ok_or_else(|| "Cannot locate installed EXE".to_owned())?;

    let json = serde_json::to_string_pretty(style)
        .map_err(|e| e.to_string())?;

    // Write into a temporary file, then publish.
    let temp = path.with_extension("json.tmp");

    fs::write(&temp, json)
        .map_err(|e| format!("Settings write failed: {e}"))?;

    if path.exists() {
        fs::remove_file(&path)
            .map_err(|e| format!("Settings replace failed: {e}"))?;
    }

    fs::rename(&temp, &path)
        .map_err(|e| format!("Settings rename failed: {e}"))?;

    Ok(())
}

fn commit_style(style: Style) -> Result<(), String> {
    let mut updated = style;
    updated.sanitize();

    {
        let mut current = storage()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        *current = updated.clone();
    }

    save(&updated)
}

// LCK_V205_INLINE_COLORS
//
// These controls draw directly INSIDE the submenu.
// They do not launch a second color popup, so selecting
// a color, dragging a slider, or editing RGB values
// does not dismiss the View menu.

fn v205_presets() -> [(&'static str,[u8;3]);8] {
    [
        ("Cyan",    [89,166,196]),
        ("Blue",    [64,117,225]),
        ("Teal",    [67,185,169]),
        ("Green",   [110,193,102]),
        ("Orange",  [238,164,96]),
        ("Gold",    [236,196,79]),
        ("Purple",  [174,126,225]),
        ("Pink",    [227,122,170]),
    ]
}

fn v205_color_controls(
    ui:&mut egui::Ui,
    color:&mut [u8;3],
)->bool {
    let mut changed=false;

    ui.label("Color presets");

    ui.horizontal_wrapped(|ui| {
        for (name,preset) in v205_presets() {
            let active=*color==preset;

            let button=egui::Button::new(
                if active {"✓"}else{" "}
            )
            .fill(egui::Color32::from_rgb(
                preset[0],preset[1],preset[2]
            ))
            .min_size(egui::vec2(27.0,25.0));

            if ui.add(button)
                .on_hover_text(name)
                .clicked()
            {
                *color=preset;
                changed=true;
            }
        }
    });

    ui.horizontal(|ui| {
        ui.label("Current");

        let preview=egui::Color32::from_rgb(
            color[0],color[1],color[2]
        );

        ui.colored_label(
            preview,
            format!(
                "#{:02X}{:02X}{:02X}",
                color[0],color[1],color[2]
            )
        );
    });

    // Full native egui color picker, drawn inline
    // instead of inside another popup.
    ui.collapsing("Custom color (HSV / RGB)",|ui| {
        let mut custom=egui::Color32::from_rgb(
            color[0],color[1],color[2]
        );

        if egui::widgets::color_picker::color_picker_color32(
            ui,
            &mut custom,
            egui::widgets::color_picker::Alpha::Opaque,
        ) {
            *color=[
                custom.r(),
                custom.g(),
                custom.b(),
            ];

            changed=true;
        }

        ui.separator();
        ui.label("Precise RGB channels");

        ui.horizontal(|ui| {
            for (index,label) in [
                "R ","G ","B "
            ].iter().enumerate() {
                changed |= ui.add(
                    egui::DragValue::new(
                        &mut color[index]
                    )
                    .range(0..=255)
                    .prefix(*label)
                    .speed(1.0)
                ).changed();
            }
        });
    });

    changed
}

pub(crate) fn grid_controls(ui:&mut egui::Ui)->bool {
    let mut style=snapshot();
    let mut changed=false;

    ui.separator();
    ui.strong("Grid appearance");

    changed |= v205_color_controls(
        ui,
        &mut style.grid_color
    );

    ui.separator();

    changed |= ui.add(
        egui::Slider::new(
            &mut style.grid_transparency,
            0.0..=1.0
        )
        .text("Transparency")
    ).changed();

    ui.small("0 = solid  |  1 = invisible");

    if ui.small_button(
        "Reset grid appearance"
    ).clicked() {
        let defaults=Style::default();

        style.grid_color=defaults.grid_color;
        style.grid_transparency=
            defaults.grid_transparency;

        changed=true;
    }

    if changed {
        if let Err(error)=commit_style(style) {
            ui.colored_label(
                egui::Color32::RED,
                format!("Grid settings save failed: {error}")
            );
        }
    }

    changed
}

pub(crate) fn collision_controls(ui:&mut egui::Ui)->bool {
    let mut style=snapshot();
    let mut changed=false;

    ui.separator();
    ui.strong("Collision appearance");

    changed |= v205_color_controls(
        ui,
        &mut style.collision_color
    );

    ui.separator();

    changed |= ui.add(
        egui::Slider::new(
            &mut style.collision_transparency,
            0.0..=1.0
        )
        .text("Transparency")
    ).changed();

    ui.small("0 = solid  |  1 = invisible");
    ui.small("Added boxes use a lighter shade.");
    ui.small("Selected boxes retain gold highlighting.");

    if ui.small_button(
        "Reset collision appearance"
    ).clicked() {
        let defaults=Style::default();

        style.collision_color=
            defaults.collision_color;

        style.collision_transparency=
            defaults.collision_transparency;

        changed=true;
    }

    if changed {
        if let Err(error)=commit_style(style) {
            ui.colored_label(
                egui::Color32::RED,
                format!("Collision settings save failed: {error}")
            );
        }
    }

    changed
}

#[cfg(test)]
mod v205_color_picker_tests {
    use super::*;

    #[test]
    fn v205_palette_has_unique_colors() {
        let presets=v205_presets();

        assert_eq!(presets.len(),8);

        let unique=presets.iter()
            .map(|(_,rgb)|*rgb)
            .collect::<std::collections::HashSet<_>>();

        assert_eq!(unique.len(),8);
    }

    #[test]
    fn v205_palette_contains_both_default_colors() {
        let presets=v205_presets();
        let defaults=Style::default();

        assert!(
            presets.iter().any(|(_,rgb)|
                *rgb==defaults.grid_color
            )
        );

        assert!(
            presets.iter().any(|(_,rgb)|
                *rgb==defaults.collision_color
            )
        );
    }
}

#[cfg(test)]
mod v201_overlay_style_tests {
    use super::*;

    #[test]
    fn defaults_have_visible_translucent_fill() {
        let style = Style::default();

        assert!(style.grid_opacity() > 0.0);
        assert!(style.grid_opacity() < 1.0);

        assert!(style.collision_opacity() > 0.0);
        assert!(style.collision_opacity() < 1.0);
    }

    #[test]
    fn transparency_is_clamped() {
        let mut style = Style::default();

        style.grid_transparency = 2.0;
        style.collision_transparency = -1.0;
        style.sanitize();

        assert_eq!(style.grid_opacity(), 0.0);
        assert_eq!(style.collision_opacity(), 1.0);
    }

    #[test]
    fn overlays_are_non_pickable_ids() {
        for id in [
            u32::MAX - 1,
            u32::MAX - 3,
            u32::MAX - 4,
            u32::MAX - 5,
            u32::MAX - 7,
            u32::MAX - 8,
        ] {
            assert!(is_overlay_id(id));
        }

        assert!(!is_overlay_id(42));
    }

    #[test]
    fn opacity_controls_all_overlay_faces() {
        let mut style = Style::default();

        style.grid_transparency = 1.0;
        style.collision_transparency = 1.0;

        for id in [
            u32::MAX - 1,
            u32::MAX - 3,
            u32::MAX - 4,
            u32::MAX - 5,
        ] {
            assert_eq!(style.alpha_for_id(id), Some(0.0));
        }

        assert_eq!(
            style.alpha_for_id(u32::MAX - 7),
            Some(1.0)
        );
    }

    #[test]
    fn settings_roundtrip() {
        let mut style = Style::default();

        style.grid_color = [80, 120, 200];
        style.collision_color = [240, 100, 60];
        style.grid_transparency = 0.4;

        let json = serde_json::to_string(&style).unwrap();
        let decoded: Style = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.grid_color, [80, 120, 200]);
        assert_eq!(decoded.collision_color, [240, 100, 60]);
        assert_eq!(decoded.grid_transparency, 0.4);
    }
}