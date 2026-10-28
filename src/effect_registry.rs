use std::sync::mpsc::Sender;

use ratatui::{
    buffer::Buffer,
    layout::{Margin, Rect},
    style::Color,
};
use tachyonfx::{
    CellFilter, Duration, Effect, EffectManager, EffectTimer, Interpolation::*, Motion, fx,
};

use crate::{
    config::EffectsSection,
    dispatcher::Dispatcher,
    event::SlimEvent,
    gruvbox::Gruvbox::{self, *},
    ui::popup::utility::CenteredShrink,
};

pub struct EffectRegistry {
    effects: EffectManager<FxKey>,
    sender: Sender<SlimEvent>,
    pub effects_config: EffectsSection,
    active_until_ms: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Ord, PartialOrd)]
pub enum FxKey {
    #[default]
    PopupOpen,
    PopupClose,
    Notice,
    RowHighlight,
}

impl EffectRegistry {
    pub fn new(sender: Sender<SlimEvent>, effects_config: EffectsSection) -> Self {
        Self {
            effects: EffectManager::default(),
            sender,
            effects_config,
            active_until_ms: 0,
        }
    }

    pub fn is_running(&self) -> bool {
        self.active_until_ms > 0
    }

    pub fn process_effects(&mut self, elapsed: Duration, buf: &mut Buffer, area: Rect) {
        self.active_until_ms = self.active_until_ms.saturating_sub(elapsed.as_millis());
        self.effects.process_effects(elapsed, buf, area);
    }

    pub fn register_popup_open(&mut self, area: Rect) {
        if !self.effects_config.animations_enabled {
            return;
        }
        self.active_until_ms = self.active_until_ms.max(250);
        let fx = open_window().with_area(area);
        self.effects.add_unique_effect(FxKey::PopupOpen, fx);
    }

    pub fn register_popup_close(&mut self, area: Rect) {
        if !self.effects_config.animations_enabled {
            return;
        }
        self.active_until_ms = self.active_until_ms.max(200);
        let fx = close_window().with_area(area);
        self.effects.add_unique_effect(FxKey::PopupClose, fx);
    }

    pub fn register_row_highlight(&mut self, area: Rect) {
        if !self.effects_config.animations_enabled {
            return;
        }
        self.active_until_ms = self.active_until_ms.max(380);
        let c: Color = Gruvbox::Orange.into();
        let fx = fx::fade_from_fg(c, (350, QuadOut)).with_area(area);
        self.effects.add_unique_effect(FxKey::RowHighlight, fx);
    }

    pub fn register_notification_effect(&mut self, area: Rect) {
        let sender = self.sender.clone();
        let hold_ms = self.effects_config.in_app_notification_timeout_ms.max(1000) as u32;
        self.active_until_ms = self.active_until_ms.max(hold_ms + 900);

        if !self.effects_config.animations_enabled {
            let fx = fx::sequence(&[
                fx::sleep(hold_ms),
                fx::effect_fn_buf((), 1, move |_, _, _| {
                    sender.dispatch(SlimEvent::NotificationDismiss);
                }),
            ])
            .with_area(area);
            self.effects.add_unique_effect(FxKey::Notice, fx);
            return;
        }

        let move_in_window = fx::slide_in(Motion::UpToDown, 10, 0, Dark0, (350, QuadInOut));
        let move_out_window = move_in_window.clone().reversed();

        let clear_area_fx = fx::effect_fn_buf((), hold_ms, move |_, _, _| {});
        let dismiss_fx = fx::effect_fn_buf((), 1, move |_, _, _| {
            sender.dispatch(SlimEvent::NotificationDismiss);
        });

        let c: Color = Gruvbox::YellowBright.into();
        let text_area = area.inner_centered(area.width.saturating_sub(2), 1);

        let fx = fx::sequence(&[
            fx::parallel(&[
                move_in_window,
                fx::fade_from(Dark0, Dark0, (400, QuadInOut)),
                fx::sweep_in(Motion::RightToLeft, 15, 0, Dark0, (600, QuadOut))
                    .with_area(text_area),
                fx::sequence(&[fx::sleep(200), fx::fade_from_fg(c, (400, QuadOut))])
                    .with_area(text_area),
            ]),
            clear_area_fx,
            dismiss_fx,
            move_out_window,
        ])
        .with_area(area);

        self.effects.add_unique_effect(FxKey::Notice, fx);
    }
}

pub fn open_window() -> Effect {
    let margin = Margin::new(1, 1);
    let border_text = CellFilter::AllOf(vec![CellFilter::Outer(margin), CellFilter::Text]);
    let popup_content = CellFilter::Inner(margin);

    let timer = EffectTimer::from_ms(220, CircOut);
    fx::parallel(&[
        fx::coalesce((220, QuintIn)).with_filter(border_text),
        fx::fade_from(Dark0, Dark0, timer).with_filter(popup_content),
    ])
}

pub fn close_window() -> Effect {
    let margin = Margin::new(1, 1);
    let border_text = CellFilter::AllOf(vec![CellFilter::Outer(margin), CellFilter::Text]);
    let popup_content = CellFilter::Inner(margin);

    fx::parallel(&[
        fx::dissolve((180, CircOut)).with_filter(border_text),
        fx::fade_to(Dark0, Dark0, (180, CircOut)).with_filter(popup_content),
    ])
}
