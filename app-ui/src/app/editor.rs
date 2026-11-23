//! Theme editor: a panel docked at the right of the window that changes
//! every colour and option of a theme while the rest of slogger, still
//! live beside it, shows the result.
//!
//! What is edited is always a named copy held in a [`Spec`]: nothing ties
//! it to the theme it was taken from, and the themes slogger ships are
//! never changed. Saving writes a theme file to the themes directory.

use std::path::PathBuf;

use iced::widget::{
    Space, button, checkbox, column, container, pick_list, row, scrollable, slider, text,
    text_input, themer,
};
use iced::{Border, Color, Element, Length, Task, Theme};

use super::message::Message;
use super::state::App;
use super::theme::{self, Role, Spec, StatusLook, ThemeEntry};

/// What the theme under edit is called while it is being previewed. It
/// is never saved under this name.
const PREVIEW_NAME: &str = "Theme being edited";

const PANEL_WIDTH: f32 = 372.0;

/// An edit in progress (`App::theme_editor`).
#[derive(Debug, Clone)]
pub struct ThemeEditor {
    spec: Spec,
    /// What is typed in each colour's box; the spec only follows once it
    /// is a whole colour.
    hex: [String; Role::COUNT],
    /// The colour whose sliders are showing, with their positions (hue,
    /// saturation, lightness). Kept here because a grey has no hue to
    /// read back.
    open: Option<(Role, [f32; 3])>,
    /// The theme file being changed; `None` for a new copy.
    file: Option<PathBuf>,
    /// The theme to go back to on Cancel.
    revert: Theme,
    error: Option<String>,
}

#[derive(Debug, Clone)]
pub enum EditorMessage {
    Name(String),
    Hex(Role, String),
    /// Show or hide a colour's sliders.
    Toggle(Role),
    /// A slider moved: 0 hue, 1 saturation, 2 lightness.
    Hsl(usize, f32),
    OneInk(bool),
    Edge(f32),
    Status(StatusLook),
    Zebra(bool),
    Save,
    Cancel,
}

impl ThemeEditor {
    fn new(spec: Spec, file: Option<PathBuf>, revert: Theme) -> Self {
        let hex = Role::ALL.map(|role| theme::to_hex(spec.get(role)));
        ThemeEditor { spec, hex, open: None, file, revert, error: None }
    }

    fn preview(&self) -> Theme {
        self.spec.theme_named(PREVIEW_NAME.to_owned())
    }

    fn set(&mut self, role: Role, color: Color) {
        self.spec.set(role, color);
        self.hex[role as usize] = theme::to_hex(color);
    }
}

/// A name for a copy of `name` that no theme has yet.
fn copy_name(name: &str, themes: &[ThemeEntry]) -> String {
    let base = name.trim_end_matches(" (custom)");
    let taken = |candidate: &str| themes.iter().any(|e| e.theme.to_string() == candidate);
    let first = format!("{base} copy");
    if !taken(&first) {
        return first;
    }
    (2..)
        .map(|n| format!("{base} copy {n}"))
        .find(|candidate| !taken(candidate))
        .unwrap_or(first)
}

impl App {
    /// Opens the editor on the theme in use: on the file itself when
    /// that is a theme of the user's own and `copy` is false, otherwise
    /// on a new copy.
    pub(super) fn edit_theme(&mut self, copy: bool) -> Task<Message> {
        let current = self.theme.to_string();
        let file = self
            .themes
            .iter()
            .find(|entry| entry.theme.to_string() == current)
            .and_then(|entry| entry.file.clone())
            .filter(|_| !copy);
        let mut spec = Spec::from_theme(&self.theme);
        if file.is_none() {
            spec.name = copy_name(&current, &self.themes);
        }
        let editor = ThemeEditor::new(spec, file, self.theme.clone());
        self.theme = editor.preview();
        self.theme_editor = Some(editor);
        self.theme_delete_armed = false;
        self.modal = None;
        Task::none()
    }

    /// Deletes the file of the theme in use, on the second press.
    pub(super) fn delete_theme(&mut self) -> Task<Message> {
        let current = self.theme.to_string();
        let Some(file) = self
            .themes
            .iter()
            .find(|entry| entry.theme.to_string() == current)
            .and_then(|entry| entry.file.clone())
        else {
            return Task::none();
        };
        if !self.theme_delete_armed {
            self.theme_delete_armed = true;
            return Task::none();
        }
        self.theme_delete_armed = false;
        match std::fs::remove_file(&file) {
            Ok(()) => {
                self.status = Some(format!("deleted theme '{current}'"));
                self.reload_themes();
                self.theme = theme::default_theme();
                self.save_panel_layout()
            }
            Err(e) => {
                tracing::warn!(path = %file.display(), "delete theme: {e}");
                self.status = Some(format!("delete theme error: {e}"));
                Task::none()
            }
        }
    }

    /// Re-reads the theme files, reporting any that could not be used in
    /// the status bar (they are logged where they are found).
    pub(super) fn reload_themes(&mut self) {
        let (themes, problems) = theme::load_themes();
        self.themes = themes;
        if let Some(first) = problems.first() {
            self.status = Some(match problems.len() {
                1 => first.clone(),
                n => format!("{first} (and {} more, see the log)", n - 1),
            });
        }
    }

    pub(super) fn update_theme_editor(&mut self, message: EditorMessage) -> Task<Message> {
        let Some(editor) = self.theme_editor.as_mut() else {
            return Task::none();
        };
        editor.error = None;
        match message {
            EditorMessage::Name(name) => editor.spec.name = name,
            EditorMessage::Hex(role, typed) => {
                if let Some(color) = parse_hex(&typed) {
                    editor.spec.set(role, color);
                    if let Some((open, hsl)) = editor.open.as_mut()
                        && *open == role
                    {
                        *hsl = theme::to_hsl(color);
                    }
                }
                editor.hex[role as usize] = typed;
            }
            EditorMessage::Toggle(role) => {
                editor.open = match editor.open {
                    Some((open, _)) if open == role => None,
                    _ => Some((role, theme::to_hsl(editor.spec.get(role)))),
                };
            }
            EditorMessage::Hsl(channel, value) => {
                if let Some((role, mut hsl)) = editor.open {
                    hsl[channel.min(2)] = value;
                    editor.open = Some((role, hsl));
                    editor.set(role, theme::from_hsl(hsl));
                }
            }
            EditorMessage::OneInk(on) => editor.spec.one_ink = on,
            EditorMessage::Edge(width) => editor.spec.edge = width,
            EditorMessage::Status(look) => editor.spec.status = look,
            EditorMessage::Zebra(on) => editor.spec.zebra = on,
            EditorMessage::Cancel => {
                self.theme = editor.revert.clone();
                self.theme_editor = None;
                return Task::none();
            }
            EditorMessage::Save => return self.save_theme(),
        }
        if let Some(editor) = &self.theme_editor {
            self.theme = editor.preview();
        }
        Task::none()
    }

    fn save_theme(&mut self) -> Task<Message> {
        let Some(editor) = self.theme_editor.as_mut() else {
            return Task::none();
        };
        let name = editor.spec.name.trim().to_owned();
        if name.is_empty() {
            editor.error = Some("Give the theme a name.".into());
            return Task::none();
        }
        // Another theme may not share the name; the file being edited may
        // keep its own.
        let (themes, _) = theme::load_themes();
        let clash = themes
            .iter()
            .any(|entry| entry.theme.to_string() == name && entry.file != editor.file)
            || (editor.file.is_none() && themes.iter().any(|e| e.theme.to_string() == name));
        if clash || name == PREVIEW_NAME {
            editor.error = Some(format!("There is already a theme named {name}."));
            return Task::none();
        }
        let Some(dir) = theme::themes_dir() else {
            editor.error = Some("No configuration directory to save themes in.".into());
            return Task::none();
        };
        editor.spec.name = name.clone();
        match editor.spec.save(&dir, editor.file.as_deref()) {
            Ok(path) => {
                let revert = editor.revert.clone();
                self.theme_editor = None;
                self.status = Some(format!("saved theme '{name}' to {}", path.display()));
                self.reload_themes();
                self.theme = theme::theme_by_name(&self.themes, &name).unwrap_or(revert);
                self.save_panel_layout()
            }
            Err(e) => {
                tracing::warn!(dir = %dir.display(), "save theme: {e}");
                editor.error = Some(format!("Could not save: {e}"));
                Task::none()
            }
        }
    }
}

/// `#rrggbb` or `rrggbb`, once all six digits are there.
fn parse_hex(typed: &str) -> Option<Color> {
    let hex = typed.trim().trim_start_matches('#');
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some(Color::from_rgb8(channel(0)?, channel(2)?, channel(4)?))
}

fn send(message: EditorMessage) -> Message {
    Message::ThemeEditor(message)
}

/// The editor panel. Drawn in a fixed theme of its own so it can be read
/// whatever the colours being edited are.
pub(super) fn view(editor: &ThemeEditor) -> Element<'_, Message> {
    let spec = &editor.spec;

    let name = column![
        theme::label("NAME"),
        text_input("Theme name", &spec.name)
            .on_input(|s| send(EditorMessage::Name(s)))
            .size(14)
            .padding([4, 8]),
    ]
    .spacing(4);

    let mut colors = column![].spacing(5);
    let mut group = "";
    for role in Role::ALL {
        if role.group() != group {
            group = role.group();
            colors = colors.push(Space::new().height(4)).push(theme::label(group));
        }
        if role == Role::SelectionText {
            colors = colors.push(
                checkbox(spec.one_ink)
                    .label("One text colour on the selected row")
                    .on_toggle(|on| send(EditorMessage::OneInk(on)))
                    .size(15)
                    .text_size(13),
            );
            if !spec.one_ink {
                continue;
            }
        }
        colors = colors.push(color_row(editor, role));
        if let Some((open, hsl)) = editor.open
            && open == role
        {
            colors = colors.push(sliders(hsl));
        }
    }

    let options = column![
        Space::new().height(4),
        theme::label("OPTIONS"),
        row![
            text("Border width").size(13).width(Length::Fill),
            slider(0.0..=theme::EDGE_MAX, spec.edge, |w| send(EditorMessage::Edge(w)))
                .step(0.5_f32)
                .width(120),
            text(format!("{:.1}", spec.edge)).size(13).font(theme::MONO).width(30),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center),
        row![
            text("Status shown as").size(13).width(Length::Fill),
            pick_list(StatusLook::ALL, Some(spec.status), |look| send(EditorMessage::Status(look)))
                .text_size(13)
                .padding([2, 8])
                .width(158),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center),
        checkbox(spec.zebra)
            .label("Shade alternate table rows")
            .on_toggle(|on| send(EditorMessage::Zebra(on)))
            .size(15)
            .text_size(13),
    ]
    .spacing(6);

    let warnings = spec.warnings();
    let mut notes = column![Space::new().height(4), theme::label("HARD TO READ")].spacing(4);
    if warnings.is_empty() {
        notes = notes.push(text("Nothing: every pairing checked can be read.").size(12));
    }
    for warning in warnings {
        notes = notes.push(text(warning).size(12).style(text::danger));
    }

    let body = scrollable(
        container(column![colors, options, notes].spacing(8)).padding(iced::Padding {
            right: 14.0,
            ..iced::Padding::ZERO
        }),
    )
    .height(Length::Fill);

    let mut footer = column![].spacing(6);
    if let Some(error) = &editor.error {
        footer = footer.push(text(error).size(13).style(text::danger));
    }
    footer = footer.push(
        row![
            text(match &editor.file {
                Some(_) => "Saves over this theme's file.",
                None => "Saves as a new theme.",
            })
            .size(12)
            .width(Length::Fill),
            button(text("Cancel").size(13))
                .padding([4, 12])
                .on_press(send(EditorMessage::Cancel))
                .style(theme::push_button),
            button(text("Save").size(13))
                .padding([4, 14])
                .on_press(send(EditorMessage::Save))
                .style(button::primary),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center),
    );

    let panel = container(
        column![text("Edit theme").size(16), name, body, footer].spacing(10),
    )
    .padding(12)
    .width(Length::Fixed(PANEL_WIDTH))
    .height(Length::Fill)
    .style(theme::panel_style);

    themer(Some(theme::editor_theme()), panel).into()
}

/// One colour: a swatch that opens its sliders, its name, and its hex
/// value.
fn color_row(editor: &ThemeEditor, role: Role) -> Element<'_, Message> {
    let color = editor.spec.get(role);
    let typed = &editor.hex[role as usize];
    let is_open = editor.open.is_some_and(|(open, _)| open == role);
    let swatch = button(Space::new().width(30).height(18))
        .padding(0)
        .on_press(send(EditorMessage::Toggle(role)))
        .style(move |app: &Theme, _| button::Style {
            background: Some(color.into()),
            border: Border {
                color: if is_open {
                    app.palette().primary
                } else {
                    app.extended_palette().background.strong.color
                },
                width: if is_open { 2.0 } else { 1.0 },
                radius: 3.0.into(),
            },
            ..button::Style::default()
        });
    let valid = parse_hex(typed).is_some();
    row![
        swatch,
        text(role.label()).size(13).width(Length::Fill),
        text(if valid { "" } else { "?" }).size(13).style(text::danger),
        text_input("#rrggbb", typed)
            .on_input(move |s| send(EditorMessage::Hex(role, s)))
            .font(theme::MONO)
            .size(13)
            .padding([2, 6])
            .width(84),
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center)
    .into()
}

fn sliders<'a>([h, s, l]: [f32; 3]) -> Element<'a, Message> {
    let one = |name: &'static str, channel: usize, value: f32, max: f32, shown: String| {
        row![
            text(name).size(12).width(70),
            slider(0.0..=max, value, move |v| send(EditorMessage::Hsl(channel, v)))
                .step(max / 360.0),
            text(shown).size(12).font(theme::MONO).width(36),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center)
    };
    container(
        column![
            one("Hue", 0, h, 359.0, format!("{h:.0}°")),
            one("Saturation", 1, s, 1.0, format!("{:.0}%", s * 100.0)),
            one("Lightness", 2, l, 1.0, format!("{:.0}%", l * 100.0)),
        ]
        .spacing(4),
    )
    .padding(iced::Padding { left: 38.0, top: 2.0, bottom: 6.0, right: 0.0 })
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editing(app: &App) -> &ThemeEditor {
        app.theme_editor.as_ref().expect("editor open")
    }

    #[test]
    fn editing_a_shipped_theme_starts_a_named_copy_and_previews_changes() {
        let (mut app, _) = App::init();
        let before = app.theme.clone();
        let _ = app.edit_theme(false);
        assert_eq!(editing(&app).spec.name, format!("{before} copy"));
        assert!(editing(&app).file.is_none());
        assert_eq!(app.theme.to_string(), PREVIEW_NAME);
        // The preview's colours are checked through the spec: tests run
        // side by side and share the one preview name.
        let pane = theme::hues(&before).pane;
        assert_eq!(editing(&app).spec.get(Role::Pane), pane);

        let _ = app.update_theme_editor(EditorMessage::Hex(Role::Pane, "#12345".into()));
        assert_eq!(editing(&app).spec.get(Role::Pane), pane, "not a colour yet");
        let _ = app.update_theme_editor(EditorMessage::Hex(Role::Pane, "#123456".into()));
        assert_eq!(theme::to_hex(editing(&app).spec.get(Role::Pane)), "#123456");

        let _ = app.update_theme_editor(EditorMessage::Toggle(Role::Field));
        let _ = app.update_theme_editor(EditorMessage::Hsl(2, 0.0));
        assert_eq!(theme::to_hex(app.theme.palette().background), "#000000");
        assert_eq!(editing(&app).hex[Role::Field as usize], "#000000");

        let _ = app.update_theme_editor(EditorMessage::Cancel);
        assert!(app.theme_editor.is_none());
        assert_eq!(app.theme.to_string(), before.to_string());
    }

    #[test]
    fn a_theme_cannot_be_saved_without_a_name_or_under_a_taken_one() {
        let (mut app, _) = App::init();
        let _ = app.edit_theme(true);
        let _ = app.update_theme_editor(EditorMessage::Name("  ".into()));
        let _ = app.update_theme_editor(EditorMessage::Save);
        assert!(editing(&app).error.is_some());

        let _ = app.update_theme_editor(EditorMessage::Name("Paper & Blue".into()));
        let _ = app.update_theme_editor(EditorMessage::Save);
        assert!(editing(&app).error.as_deref().is_some_and(|e| e.contains("already")));
    }

    #[test]
    fn copies_get_names_nothing_else_has() {
        let themes = theme::all_themes();
        assert_eq!(copy_name("Fog & Blue", &themes), "Fog & Blue copy");
        assert_eq!(copy_name("Mine (custom)", &themes), "Mine copy");
    }
}
