//! Native Android two-thumb gameplay controls.
use bevy::window::{PrimaryWindow, WindowFocused};
use bevy::{
    input::touch::{TouchInput, TouchPhase, Touches},
    prelude::*,
    ui::FocusPolicy,
};

use super::{Phase, Run};

#[derive(Resource, Default)]
pub(super) struct TouchControls {
    stick: Option<u64>,
    pub(super) direction: Vec2,
    pub(super) jump: bool,
    pub(super) mobility: bool,
    pub(super) zoom: f32,
}

#[derive(Component)]
struct Controls;
#[derive(Component)]
struct Thumb;

#[derive(Component, Clone, Copy)]
enum Control {
    Stick,
    Jump,
    Mobility,
    ZoomIn,
    ZoomOut,
    Pause,
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<TouchControls>()
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            read_touches
                .before(super::hero::aim_cursor)
                .in_set(benilla_world::schedule::WorldStage::Input),
        );
}

fn setup(mut commands: Commands) {
    commands
        .spawn((
            Controls,
            Node {
                display: Display::None,
                ..super::perk_shop::full_panel()
            },
            FocusPolicy::Pass,
            GlobalZIndex(15),
        ))
        .with_children(|root| {
            root.spawn((
                Control::Stick,
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(54.0),
                    bottom: Val::Px(68.0),
                    width: Val::Px(226.0),
                    height: Val::Px(226.0),
                    border: UiRect::all(Val::Px(3.0)),
                    border_radius: BorderRadius::MAX,
                    ..default()
                },
                BackgroundColor(Color::srgba(0.035, 0.025, 0.02, 0.47)),
                BorderColor::all(Color::srgba(0.91, 0.73, 0.45, 0.70)),
                FocusPolicy::Pass,
            ))
            .with_child((
                Thumb,
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Percent(50.0),
                    top: Val::Percent(50.0),
                    margin: UiRect::new(Val::Px(-39.0), Val::ZERO, Val::Px(-39.0), Val::ZERO),
                    width: Val::Px(78.0),
                    height: Val::Px(78.0),
                    border_radius: BorderRadius::MAX,
                    ..default()
                },
                BackgroundColor(Color::srgba(0.86, 0.69, 0.45, 0.80)),
                FocusPolicy::Pass,
            ));
            for (control, label, right, bottom, size, font) in [
                (Control::Jump, "Jump", 58.0, 72.0, 100.0, 26.0),
                (Control::Mobility, "Skill", 176.0, 120.0, 100.0, 26.0),
                (Control::ZoomIn, "+", 50.0, 324.0, 74.0, 36.0),
                (Control::ZoomOut, "−", 50.0, 238.0, 74.0, 36.0),
                (Control::Pause, "II", 48.0, 412.0, 78.0, 29.0),
            ] {
                root.spawn((
                    control,
                    Node {
                        position_type: PositionType::Absolute,
                        right: Val::Px(right),
                        bottom: Val::Px(bottom),
                        width: Val::Px(size),
                        height: Val::Px(size),
                        border: UiRect::all(Val::Px(2.0)),
                        border_radius: BorderRadius::MAX,
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.12, 0.075, 0.042, 0.77)),
                    BorderColor::all(Color::srgba(0.91, 0.73, 0.45, 0.70)),
                    FocusPolicy::Pass,
                ))
                .with_child((
                    Text::new(label),
                    TextFont::from_font_size(font),
                    TextColor(Color::srgb(1.0, 0.89, 0.68)),
                    FocusPolicy::Pass,
                ));
            }
        });
}

#[allow(clippy::too_many_arguments)]
fn read_touches(
    mut events: MessageReader<TouchInput>,
    mut focus: MessageReader<WindowFocused>,
    mut touches: ResMut<Touches>,
    windows: Query<(Entity, &Window), With<PrimaryWindow>>,
    phase: Res<State<Phase>>,
    loading: Res<crate::loading_screen::LoadingScreen>,
    run: Res<Run>,
    mut next: ResMut<NextState<Phase>>,
    mut touch: ResMut<TouchControls>,
    mut roots: Query<&mut Node, (With<Controls>, Without<Thumb>)>,
    controls: Query<(&Control, &ComputedNode, &UiGlobalTransform)>,
    mut thumb: Query<&mut Node, With<Thumb>>,
) {
    touch.jump = false;
    touch.mobility = false;
    touch.zoom = 0.0;
    let lost_focus = focus.read().any(|event| !event.focused);
    let active = *phase.get() == Phase::Playing && !loading.covering() && run.dead_t.is_none();
    for mut root in &mut roots {
        let display = if active { Display::Flex } else { Display::None };
        if root.display != display {
            root.display = display;
        }
    }
    if !active || lost_focus {
        if let Some(stick) = touch.stick {
            touches.release(stick);
        }
        *touch = TouchControls::default();
        events.clear();
        if lost_focus && active {
            next.set(Phase::Paused);
        }
    } else if let Ok((entity, window)) = windows.single() {
        for event in events.read().filter(|event| event.window == entity) {
            let point = event.position * window.scale_factor();
            match event.phase {
                TouchPhase::Started => {
                    for (&control, node, &transform) in &controls {
                        if !node.contains_point(transform, point) {
                            continue;
                        }
                        match control {
                            Control::Stick if touch.stick.is_none() => {
                                touch.stick = Some(event.id);
                                touch.direction = stick_direction(point, node, transform);
                            }
                            Control::Stick => {}
                            Control::Jump => touch.jump = true,
                            Control::Mobility => touch.mobility = true,
                            Control::ZoomIn => touch.zoom += 1.0,
                            Control::ZoomOut => touch.zoom -= 1.0,
                            Control::Pause => next.set(Phase::Paused),
                        }
                        break;
                    }
                }
                TouchPhase::Moved if touch.stick == Some(event.id) => {
                    if let Some((_, node, &transform)) = controls
                        .iter()
                        .find(|(control, ..)| matches!(control, Control::Stick))
                    {
                        touch.direction = stick_direction(point, node, transform);
                    }
                }
                TouchPhase::Ended | TouchPhase::Canceled if touch.stick == Some(event.id) => {
                    touch.stick = None;
                    touch.direction = Vec2::ZERO;
                }
                _ => {}
            }
        }
    }
    if let Ok(mut thumb) = thumb.single_mut() {
        let left = Val::Percent(50.0 + touch.direction.x * 30.0);
        let top = Val::Percent(50.0 + touch.direction.y * 30.0);
        if thumb.left != left || thumb.top != top {
            thumb.left = left;
            thumb.top = top;
        }
    }
}

fn stick_direction(point: Vec2, node: &ComputedNode, transform: UiGlobalTransform) -> Vec2 {
    let raw = (point - transform * Vec2::ZERO) / (node.size().x * 0.32).max(1.0);
    let length = raw.length();
    const DEAD_ZONE: f32 = 0.14;
    if length <= DEAD_ZONE {
        Vec2::ZERO
    } else {
        raw / length * ((length - DEAD_ZONE) / (1.0 - DEAD_ZONE)).min(1.0)
    }
}
