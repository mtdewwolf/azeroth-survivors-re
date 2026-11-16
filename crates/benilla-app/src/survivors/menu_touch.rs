//! Glue buttons activate on touch release over the same target.

use bevy::input::touch::{TouchInput, TouchPhase};
use bevy::prelude::*;
use bevy::window::{PrimaryWindow, WindowFocused};

use super::{menu::MenuAction, Phase};
use crate::{glue::widgets::GlueDisabled, loading_screen::LoadingScreen};

#[derive(Resource, Default)]
pub(super) struct MenuTouch {
    pressed: Option<(u64, Entity)>,
    pub(super) clicked: Option<Entity>,
}

#[allow(clippy::type_complexity)]
pub(super) fn read_menu_taps(
    mut events: MessageReader<TouchInput>,
    mut focus: MessageReader<WindowFocused>,
    windows: Query<(Entity, &Window), With<PrimaryWindow>>,
    phase: Res<State<Phase>>,
    loading: Res<LoadingScreen>,
    targets: Query<
        (
            Entity,
            &ComputedNode,
            &UiGlobalTransform,
            &InheritedVisibility,
            Option<&GlueDisabled>,
        ),
        With<MenuAction>,
    >,
    mut touch: ResMut<MenuTouch>,
) {
    touch.clicked = None;
    let lost_focus = focus.read().any(|event| !event.focused);
    let Ok((window_entity, window)) = windows.single() else {
        touch.pressed = None;
        events.clear();
        return;
    };
    if *phase.get() != Phase::Menu || loading.covering() || lost_focus || !window.focused {
        touch.pressed = None;
        events.clear();
        return;
    }
    for event in events.read().filter(|event| event.window == window_entity) {
        let target_at_event = || {
            let point = event.position * window.scale_factor();
            targets
                .iter()
                .filter(|(_, node, transform, visibility, disabled)| {
                    visibility.get()
                        && !disabled.is_some_and(|disabled| disabled.0)
                        && !node.is_empty()
                        && node.contains_point(**transform, point)
                })
                .max_by_key(|(_, node, ..)| node.stack_index())
                .map(|(entity, ..)| entity)
        };
        match (event.phase, touch.pressed) {
            (TouchPhase::Started, None) => {
                touch.pressed = target_at_event().map(|entity| (event.id, entity))
            }
            (TouchPhase::Ended, Some((id, entity))) if id == event.id => {
                touch.pressed = None;
                if target_at_event() == Some(entity) {
                    touch.clicked.get_or_insert(entity);
                }
            }
            (TouchPhase::Canceled, Some((id, _))) if id == event.id => touch.pressed = None,
            _ => {}
        }
    }
}
