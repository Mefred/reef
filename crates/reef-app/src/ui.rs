//! Reusable native wgpui components shared by Reef's workspaces.
//!
//! These helpers deliberately stay small: they compose wgpui primitives and
//! the Reef palette, but never own application state. Dialogs, forms, tables,
//! selects, status badges and empty states all use the same visual language.

use wgpui::{
    div, prelude::*, px, AnyElement, App, ClickEvent, ElementId, FontWeight, Hsla, SharedString,
    Window,
};

use crate::palette;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Accent,
    Warning,
    Danger,
}

impl Tone {
    pub fn color(self) -> Hsla {
        match self {
            Tone::Neutral => palette::text_muted(),
            Tone::Accent => palette::accent_strong(),
            Tone::Warning => palette::warning(),
            Tone::Danger => palette::danger(),
        }
    }

    pub fn border(self) -> Hsla {
        match self {
            Tone::Neutral => palette::panel_border(),
            Tone::Accent => palette::accent_dim(),
            Tone::Warning => palette::warning(),
            Tone::Danger => palette::danger(),
        }
    }

    pub fn soft_bg(self) -> Hsla {
        match self {
            Tone::Accent => palette::selection(),
            Tone::Warning | Tone::Danger => palette::panel_alt(),
            Tone::Neutral => palette::panel_alt(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonTone {
    Primary,
    Secondary,
    Ghost,
    Danger,
}

pub fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    tone: ButtonTone,
    enabled: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let label: SharedString = label.into();
    let mut button = div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .gap_1()
        .h(px(34.))
        .px_3()
        .rounded_md()
        .text_sm()
        .font_weight(FontWeight::SEMIBOLD)
        .child(label);

    button = match tone {
        ButtonTone::Primary => button
            .bg(palette::accent())
            .text_color(palette::background()),
        ButtonTone::Secondary => button
            .bg(palette::panel_alt())
            .text_color(palette::text())
            .border_1()
            .border_color(palette::panel_border()),
        ButtonTone::Ghost => button.text_color(palette::text_muted()),
        ButtonTone::Danger => button
            .bg(palette::panel_alt())
            .text_color(palette::danger())
            .border_1()
            .border_color(palette::danger()),
    };

    if enabled {
        button = button
            .cursor_pointer()
            .hover(|style| style.opacity(0.82))
            .on_click(on_click);
    } else {
        button = button.opacity(0.45);
    }

    button.into_any_element()
}

pub fn small_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    tone: ButtonTone,
    enabled: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let label: SharedString = label.into();
    let mut button = div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .h(px(26.))
        .px_2()
        .rounded_md()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .child(label);

    button = match tone {
        ButtonTone::Primary => button
            .bg(palette::accent())
            .text_color(palette::background()),
        ButtonTone::Secondary => button
            .bg(palette::panel_alt())
            .text_color(palette::text())
            .border_1()
            .border_color(palette::panel_border()),
        ButtonTone::Ghost => button.text_color(palette::text_muted()),
        ButtonTone::Danger => button.text_color(palette::danger()),
    };

    if enabled {
        button = button
            .cursor_pointer()
            .hover(|style| style.bg(palette::hover()))
            .on_click(on_click);
    } else {
        button = button.opacity(0.4);
    }

    button.into_any_element()
}

pub fn page_header(
    title: impl Into<SharedString>,
    subtitle: impl Into<SharedString>,
    actions: Option<AnyElement>,
) -> AnyElement {
    let mut row = div()
        .flex()
        .items_center()
        .justify_between()
        .gap_4()
        .w_full()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .min_w(px(0.))
                .child(
                    div()
                        .text_2xl()
                        .font_weight(FontWeight::BOLD)
                        .text_color(palette::text())
                        .child(title.into()),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(palette::text_muted())
                        .child(subtitle.into()),
                ),
        );
    if let Some(actions) = actions {
        row = row.child(
            div()
                .flex()
                .flex_shrink_0()
                .items_center()
                .gap_2()
                .child(actions),
        );
    }
    row.into_any_element()
}

pub fn panel(
    title: impl Into<SharedString>,
    subtitle: impl Into<SharedString>,
    actions: Option<AnyElement>,
    body: AnyElement,
) -> AnyElement {
    let mut header = div()
        .flex()
        .items_start()
        .justify_between()
        .gap_3()
        .w_full()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .min_w(px(0.))
                .child(
                    div()
                        .text_base()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(palette::text())
                        .child(title.into()),
                )
                .child(
                    div()
                        .text_sm()
                        .line_height(px(19.))
                        .text_color(palette::text_muted())
                        .child(subtitle.into()),
                ),
        );
    if let Some(actions) = actions {
        header = header.child(
            div()
                .flex()
                .flex_shrink_0()
                .items_center()
                .gap_2()
                .child(actions),
        );
    }

    div()
        .flex()
        .flex_col()
        .gap_3()
        .w_full()
        .p_4()
        .rounded_lg()
        .border_1()
        .border_color(palette::panel_border())
        .bg(palette::panel())
        .child(header)
        .child(body)
        .into_any_element()
}

pub fn stat_card(
    label: impl Into<SharedString>,
    value: impl Into<SharedString>,
    hint: Option<impl Into<SharedString>>,
) -> AnyElement {
    let mut card = div()
        .flex()
        .flex_col()
        .gap_1()
        .flex_1()
        .min_w(px(132.))
        .p_3()
        .rounded_md()
        .border_1()
        .border_color(palette::panel_border())
        .bg(palette::panel_alt())
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(palette::text_faint())
                .child(label.into()),
        )
        .child(
            div()
                .text_lg()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(palette::text())
                .truncate()
                .child(value.into()),
        );
    if let Some(hint) = hint {
        card = card.child(
            div()
                .text_xs()
                .text_color(palette::text_faint())
                .child(hint.into()),
        );
    }
    card.into_any_element()
}

pub fn badge(label: impl Into<SharedString>, tone: Tone) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap_1()
        .px_2()
        .py_1()
        .rounded_full()
        .border_1()
        .border_color(tone.border())
        .bg(tone.soft_bg())
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .text_color(tone.color())
        .child(label.into())
        .into_any_element()
}

pub fn status_badge(label: impl Into<SharedString>, tone: Tone) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap_2()
        .text_xs()
        .text_color(tone.color())
        .child(div().size(px(7.)).rounded_full().bg(tone.color()))
        .child(label.into())
        .into_any_element()
}

pub fn section_label(value: impl Into<SharedString>) -> AnyElement {
    div()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(palette::text_faint())
        .child(value.into())
        .into_any_element()
}

pub fn field_label(value: impl Into<SharedString>) -> AnyElement {
    div()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(palette::text_muted())
        .child(value.into())
        .into_any_element()
}

pub fn form_field(
    label: impl Into<SharedString>,
    control: AnyElement,
    hint: Option<impl Into<SharedString>>,
) -> AnyElement {
    let mut field = div()
        .flex()
        .flex_col()
        .gap_1()
        .w_full()
        .child(field_label(label))
        .child(control);
    if let Some(hint) = hint {
        field = field.child(
            div()
                .text_xs()
                .line_height(px(16.))
                .text_color(palette::text_faint())
                .child(hint.into()),
        );
    }
    field.into_any_element()
}

pub fn inline_note(message: impl Into<SharedString>, tone: Tone) -> AnyElement {
    div()
        .flex()
        .items_start()
        .gap_2()
        .w_full()
        .p_3()
        .rounded_md()
        .border_1()
        .border_color(tone.border())
        .bg(tone.soft_bg())
        .text_sm()
        .line_height(px(19.))
        .text_color(tone.color())
        .child(message.into())
        .into_any_element()
}

pub fn error_panel(message: impl Into<SharedString>) -> AnyElement {
    inline_note(message, Tone::Danger)
}

pub fn warning_panel(message: impl Into<SharedString>) -> AnyElement {
    inline_note(message, Tone::Warning)
}

pub fn empty_state(
    title: impl Into<SharedString>,
    message: impl Into<SharedString>,
    action: Option<AnyElement>,
) -> AnyElement {
    let mut body = div()
        .flex()
        .flex_col()
        .items_center()
        .gap_2()
        .w_full()
        .py_8()
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(palette::text())
                .child(title.into()),
        )
        .child(
            div()
                .max_w(px(460.))
                .text_center()
                .text_sm()
                .line_height(px(20.))
                .text_color(palette::text_muted())
                .child(message.into()),
        );
    if let Some(action) = action {
        body = body.child(action);
    }
    body.into_any_element()
}

pub fn code_block(
    id: impl Into<ElementId>,
    value: impl Into<SharedString>,
    max_height: f32,
) -> AnyElement {
    div()
        .id(id)
        .w_full()
        .max_h(px(max_height))
        .overflow_y_scroll()
        .p_3()
        .rounded_md()
        .border_1()
        .border_color(palette::panel_border())
        .bg(palette::terminal_bg())
        .font_family(".SystemMonospaceFont")
        .text_xs()
        .line_height(px(18.))
        .text_color(palette::text())
        .whitespace_normal()
        .child(value.into())
        .into_any_element()
}

pub fn divider() -> AnyElement {
    div()
        .h(px(1.))
        .w_full()
        .bg(palette::panel_border())
        .into_any_element()
}

pub fn info_item(label: impl Into<SharedString>, value: impl Into<SharedString>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .min_w(px(120.))
        .child(
            div()
                .text_xs()
                .text_color(palette::text_faint())
                .child(label.into()),
        )
        .child(
            div()
                .text_sm()
                .text_color(palette::text())
                .child(value.into()),
        )
        .into_any_element()
}

pub fn cell_text(value: impl Into<SharedString>, muted: bool, semibold: bool) -> AnyElement {
    let mut cell = div()
        .text_sm()
        .truncate()
        .text_color(if muted {
            palette::text_muted()
        } else {
            palette::text()
        })
        .child(value.into());
    if semibold {
        cell = cell.font_weight(FontWeight::SEMIBOLD);
    }
    cell.into_any_element()
}

pub fn mono_cell(value: impl Into<SharedString>, muted: bool) -> AnyElement {
    div()
        .font_family(".SystemMonospaceFont")
        .text_xs()
        .truncate()
        .text_color(if muted {
            palette::text_muted()
        } else {
            palette::text()
        })
        .child(value.into())
        .into_any_element()
}

#[derive(Clone, Copy, Debug)]
pub struct Column {
    pub label: &'static str,
    pub width: f32,
    pub grow: bool,
}

impl Column {
    pub const fn fixed(label: &'static str, width: f32) -> Self {
        Self {
            label,
            width,
            grow: false,
        }
    }

    pub const fn grow(label: &'static str, min_width: f32) -> Self {
        Self {
            label,
            width: min_width,
            grow: true,
        }
    }
}

fn column_container(column: Column) -> wgpui::Div {
    let div = div().min_w(px(0.));
    if column.grow {
        div.flex_1().min_w(px(column.width))
    } else {
        div.w(px(column.width)).flex_shrink_0()
    }
}

pub fn table_header(columns: &[Column]) -> AnyElement {
    let mut header = div()
        .flex()
        .flex_row()
        .items_center()
        .gap_3()
        .w_full()
        .px_3()
        .py_2()
        .rounded_md()
        .bg(palette::panel_alt());
    for column in columns {
        header = header.child(
            column_container(*column).child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(palette::text_faint())
                    .truncate()
                    .child(column.label),
            ),
        );
    }
    header.into_any_element()
}

pub fn table_row(cells: Vec<AnyElement>, columns: &[Column], highlighted: bool) -> AnyElement {
    let mut row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap_3()
        .w_full()
        .px_3()
        .py_2()
        .border_b_1()
        .border_color(palette::panel_border());
    if highlighted {
        row = row.bg(palette::selection());
    }
    for (cell, column) in cells.into_iter().zip(columns.iter()) {
        row = row.child(column_container(*column).child(cell));
    }
    row.into_any_element()
}

pub fn select_trigger(
    id: impl Into<ElementId>,
    value: Option<&str>,
    placeholder: &str,
    open: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let label = value.filter(|value| !value.is_empty());
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .w_full()
        .h(px(34.))
        .px_3()
        .rounded_md()
        .border_1()
        .border_color(if open {
            palette::accent()
        } else {
            palette::input_border()
        })
        .bg(palette::input_bg())
        .cursor_pointer()
        .hover(|style| style.border_color(palette::accent_dim()))
        .child(
            div()
                .text_sm()
                .truncate()
                .text_color(if label.is_some() {
                    palette::text()
                } else {
                    palette::text_faint()
                })
                .child(label.unwrap_or(placeholder).to_string()),
        )
        .child(
            div()
                .text_xs()
                .text_color(palette::text_faint())
                .child(if open { "▴" } else { "▾" }),
        )
        .on_click(on_click)
        .into_any_element()
}

pub fn select_option(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    selected: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let label: SharedString = label.into();
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .w_full()
        .px_3()
        .py_2()
        .rounded_md()
        .cursor_pointer()
        .text_sm()
        .text_color(if selected {
            palette::accent_strong()
        } else {
            palette::text()
        })
        .hover(|style| style.bg(palette::hover()))
        .child(label)
        .child(
            div()
                .text_xs()
                .text_color(palette::accent())
                .child(if selected { "✓" } else { "" }),
        )
        .on_click(on_click)
        .into_any_element()
}

pub fn toggle(
    id: impl Into<ElementId>,
    checked: bool,
    label: impl Into<SharedString>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .child(
            div()
                .size(px(18.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_md()
                .border_1()
                .border_color(if checked {
                    palette::accent()
                } else {
                    palette::input_border()
                })
                .bg(if checked {
                    palette::accent_dim()
                } else {
                    palette::input_bg()
                })
                .text_xs()
                .text_color(palette::accent_strong())
                .child(if checked { "✓" } else { "" }),
        )
        .child(
            div()
                .text_sm()
                .text_color(palette::text_muted())
                .child(label.into()),
        )
        .on_click(on_click)
        .into_any_element()
}
