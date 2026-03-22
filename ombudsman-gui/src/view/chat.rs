//! Chat panel view.

use crate::app::Message;
use crate::viewmodel::{ChatMessage, ChatRole, ChatViewModel, ConnectionStatus};
use iced::{
    Alignment, Color, Element, Fill, Length,
    widget::{
        button, column, container, row, scrollable, text, text_input,
    },
};

/// Render the chat panel.
pub fn chat_view<'a>(
    chat: &'a ChatViewModel,
    status: &'a ConnectionStatus,
) -> Element<'a, Message> {
    // Message list
    let messages: Element<Message> = if chat.messages.is_empty() {
        container(
            text("No messages yet. Connect to the server and say hello!")
                .color(Color::from_rgb(0.5, 0.5, 0.5)),
        )
        .width(Fill)
        .padding(20)
        .into()
    } else {
        let items = chat.messages.iter().map(|m| message_bubble(m));
        let col = column(items).spacing(8).padding(12);
        scrollable(col).width(Fill).height(Fill).into()
    };

    // Progress text
    let progress: Element<Message> = if let Some(hint) = &chat.progress_text {
        text(format!("⚙ {}", hint))
            .color(Color::from_rgb(0.4, 0.6, 1.0))
            .size(13)
            .into()
    } else if chat.is_loading {
        text("…thinking")
            .color(Color::from_rgb(0.5, 0.5, 0.5))
            .size(13)
            .into()
    } else {
        text("").size(13).into()
    };

    // Input row
    let input_field = text_input("Type a message…", &chat.input)
        .on_input(Message::InputChanged)
        .on_submit(Message::SendMessage)
        .padding(10)
        .width(Fill);

    let send_button = button(text("Send").size(14))
        .on_press_maybe(if !chat.input.trim().is_empty() && !chat.is_loading {
            Some(Message::SendMessage)
        } else {
            None
        })
        .padding([10, 18]);

    let input_row = row![input_field, send_button]
        .spacing(8)
        .align_y(Alignment::Center);

    // Status bar
    let status_color = match status {
        ConnectionStatus::Connected => Color::from_rgb(0.2, 0.8, 0.3),
        ConnectionStatus::Connecting => Color::from_rgb(1.0, 0.7, 0.0),
        ConnectionStatus::Error(_) => Color::from_rgb(0.9, 0.2, 0.2),
        ConnectionStatus::Disconnected => Color::from_rgb(0.5, 0.5, 0.5),
    };
    let status_label = text(format!("● {}", status.label()))
        .color(status_color)
        .size(12);

    let status_bar = container(status_label)
        .padding([4, 12])
        .width(Fill);

    column![messages, progress, input_row, status_bar]
        .spacing(6)
        .padding(12)
        .height(Fill)
        .width(Fill)
        .into()
}

/// Render a single chat message bubble.
fn message_bubble<'a>(msg: &'a ChatMessage) -> Element<'a, Message> {
    let (label, text_color, bg_color) = match msg.role {
        ChatRole::User => (
            "You",
            Color::WHITE,
            Color::from_rgb(0.2, 0.4, 0.8),
        ),
        ChatRole::Assistant => (
            "ombudsman",
            Color::BLACK,
            Color::from_rgb(0.93, 0.93, 0.93),
        ),
        ChatRole::System => (
            "System",
            Color::from_rgb(0.3, 0.3, 0.3),
            Color::from_rgb(1.0, 1.0, 0.85),
        ),
    };

    let header = text(label)
        .size(12)
        .color(match msg.role {
            ChatRole::User => Color::from_rgb(0.7, 0.8, 1.0),
            _ => Color::from_rgb(0.5, 0.5, 0.5),
        });

    let body = text(&msg.content)
        .size(14)
        .color(text_color);

    let inner = column![header, body].spacing(4);

    let bubble = container(inner)
        .padding(12)
        .style(move |_theme| {
            iced::widget::container::Style {
                background: Some(iced::Background::Color(bg_color)),
                border: iced::Border {
                    radius: 8.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            }
        });

    let width = match msg.role {
        ChatRole::User => Length::FillPortion(3),
        _ => Length::Fill,
    };

    match msg.role {
        ChatRole::User => container(bubble)
            .width(Fill)
            .align_x(iced::alignment::Horizontal::Right)
            .into(),
        _ => container(bubble).width(width).into(),
    }
}
