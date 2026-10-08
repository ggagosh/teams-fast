extern crate gpui_kit as gpui;

pub mod app;
mod dialogs;
mod html;
mod model;
mod notifications;
mod preferences;
mod realtime;
mod rich;
mod settings;
mod state;
mod teams;
mod thumbnail;
mod ui;

pub(crate) type Wake = async_channel::Sender<()>;
