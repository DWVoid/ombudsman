//! Settings modal view.

use crate::app::Message;
use crate::viewmodel::SettingsViewModel;
use iced::{
    Alignment, Color, Element, Fill,
    widget::{button, column, container, row, text, text_input},
};

/// Render the settings modal overlay.
///
/// Returns `None` if the modal should not be shown.
pub fn settings_modal<'a>(settings: &'a SettingsViewModel) -> Option<Element<'a, Message>> {
    if !settings.show {
        return None;
    }

    let title = text("⚙ Settings")
        .size(20)
        .color(Color::BLACK);

    // Server URL field
    let url_label = text("Server URL").size(13).color(Color::BLACK);
    let url_input = text_input("ws://127.0.0.1:7878/ws", &settings.draft_server_url)
        .on_input(Message::SettingsDraftServerUrl)
        .padding(8)
        .width(Fill);

    // Model field
    let model_label = text("Model override (leave empty for server default)")
        .size(13)
        .color(Color::BLACK);
    let model_input = text_input("e.g. gpt-4o", &settings.draft_model)
        .on_input(Message::SettingsDraftModel)
        .padding(8)
        .width(Fill);

    // Session key field
    let session_label = text("Session key").size(13).color(Color::BLACK);
    let session_input = text_input("gui:default", &settings.draft_session_key)
        .on_input(Message::SettingsDraftSessionKey)
        .padding(8)
        .width(Fill);

    // Buttons
    let apply_button = button(text("Apply").size(14))
        .on_press(Message::SettingsApply)
        .padding([10, 20])
        .style(|_theme, status| {
            let base = Color::from_rgb(0.2, 0.5, 0.9);
            let hovered = Color::from_rgb(0.3, 0.6, 1.0);
            iced::widget::button::Style {
                background: Some(iced::Background::Color(match status {
                    iced::widget::button::Status::Hovered => hovered,
                    _ => base,
                })),
                text_color: Color::WHITE,
                border: iced::Border {
                    radius: 6.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            }
        });

    let cancel_button = button(text("Cancel").size(14))
        .on_press(Message::SettingsCancel)
        .padding([10, 20])
        .style(|_theme, status| {
            let base = Color::from_rgb(0.85, 0.85, 0.85);
            let hovered = Color::from_rgb(0.75, 0.75, 0.75);
            iced::widget::button::Style {
                background: Some(iced::Background::Color(match status {
                    iced::widget::button::Status::Hovered => hovered,
                    _ => base,
                })),
                text_color: Color::BLACK,
                border: iced::Border {
                    radius: 6.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            }
        });

    let buttons = row![cancel_button, apply_button]
        .spacing(10)
        .align_y(Alignment::Center);

    let form = column![
        title,
        url_label,
        url_input,
        model_label,
        model_input,
        session_label,
        session_input,
        buttons,
    ]
    .spacing(10)
    .padding(30)
    .width(iced::Length::Fixed(420.0));

    let card = container(form)
        .style(|_theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(Color::WHITE)),
            border: iced::Border {
                radius: 12.0.into(),
                width: 1.0,
                color: Color::from_rgb(0.8, 0.8, 0.8),
            },
            shadow: iced::Shadow {
                color: Color::from_rgba(0.0, 0.0, 0.0, 0.25),
                offset: iced::Vector { x: 0.0, y: 4.0 },
                blur_radius: 16.0,
            },
            ..Default::default()
        });

    let overlay = container(card)
        .width(Fill)
        .height(Fill)
        .align_x(iced::alignment::Horizontal::Center)
        .align_y(iced::alignment::Vertical::Center)
        .style(|_theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(Color::from_rgba(
                0.0, 0.0, 0.0, 0.45,
            ))),
            ..Default::default()
        });

    Some(overlay.into())
}
