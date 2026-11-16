//! Character menu panel for passive purchases and starting loadout.

use bevy::{prelude::*, ui::FocusPolicy};

use super::{
    data,
    hud::{centered_bundle, icon, passive_text, text_bundle, GOLD_TONE as GOLD},
    menu::MenuAction,
    perks::{Progress, SLOTS},
};
use crate::glue::widgets::GlueDisabled;

const MUTED: Color = Color::srgb(0.77, 0.70, 0.61);
const EDGE: Color = Color::srgb(0.44, 0.34, 0.22);

#[derive(Resource, Default)]
pub(super) struct Shop {
    pub(super) open: bool,
    feedback: String,
}

#[derive(Component)]
pub(super) enum Panel {
    Picker,
    Passives,
    Characters,
}

#[derive(Component)]
pub(super) enum Caption {
    Balance,
    Feedback,
    Effect(usize),
    Button(MenuAction),
}

#[derive(Component)]
pub(super) struct ShopButton;

pub(super) fn full_panel() -> Node {
    Node {
        position_type: PositionType::Absolute,
        width: Val::Percent(100.0),
        height: Val::Percent(100.0),
        ..default()
    }
}

pub(super) fn launcher_button(ui: &mut ChildSpawnerCommands, f: &Handle<Font>) {
    button(ui, f, MenuAction::OpenPassives, "Passive upgrades", 260.0);
}

pub(super) fn build(ui: &mut ChildSpawnerCommands, f: &Handle<Font>) {
    ui.spawn((
        Panel::Passives,
        Node {
            display: Display::None,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            padding: UiRect::vertical(Val::Px(12.0)),
            ..full_panel()
        },
        Visibility::Hidden,
        BackgroundColor(Color::srgba(0.038, 0.027, 0.020, 0.97)),
    ))
    .with_children(|root| {
        root.spawn(Node {
            width: Val::Percent(94.0),
            max_width: Val::Px(1100.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(4.0),
            ..default()
        })
        .with_children(|body| {
            body.spawn(Node {
                justify_content: JustifyContent::SpaceBetween,
                align_items: AlignItems::Center,
                ..default()
            })
            .with_children(|header| {
                header
                    .spawn(Node {
                        flex_direction: FlexDirection::Column,
                        ..default()
                    })
                    .with_children(|title| {
                        title.spawn(text_bundle(f, "Passive upgrades", 32.0, GOLD));
                        title.spawn((Caption::Balance, text_bundle(f, "", 20.0, MUTED)));
                    });
                header
                    .spawn(Node {
                        column_gap: Val::Px(10.0),
                        align_items: AlignItems::Center,
                        ..default()
                    })
                    .with_children(|btns| {
                        button(btns, f, MenuAction::RespecPassives, "Respec", 190.0);
                        button(btns, f, MenuAction::ClosePassives, "Back", 120.0);
                    });
            });
            body.spawn(text_bundle(
                f,
                "Finish runs to earn points. Equip up to six meta-passives (separate from your 6 in-run passive slots).",
                19.0,
                MUTED,
            ));
            for first in (0..data::PASSIVES.len()).step_by(2) {
                body.spawn(Node {
                    column_gap: Val::Px(10.0),
                    ..default()
                })
                .with_children(|row| {
                    for index in first..(first + 2).min(data::PASSIVES.len()) {
                        card(row, f, index);
                    }
                });
            }
            body.spawn((
                Caption::Feedback,
                Node {
                    min_height: Val::Px(25.0),
                    ..default()
                },
                text_bundle(f, "", 19.0, GOLD),
            ));
        });
    });
}

fn card(row: &mut ChildSpawnerCommands, f: &Handle<Font>, index: usize) {
    row.spawn((
        Node {
            flex_basis: Val::Px(0.0),
            flex_grow: 1.0,
            min_width: Val::Px(0.0),
            padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(2.0),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(5.0)),
            ..default()
        },
        BackgroundColor(Color::srgb(0.105, 0.073, 0.048)),
        BorderColor::all(EDGE),
    ))
    .with_children(|card| {
        card.spawn(Node {
            align_items: AlignItems::Center,
            column_gap: Val::Px(9.0),
            ..default()
        })
        .with_children(|head| {
            head.spawn((icon(data::PASSIVES[index].spell, 40.0), FocusPolicy::Pass));
            head.spawn(text_bundle(f, data::PASSIVES[index].name, 21.0, GOLD));
        });
        card.spawn((Caption::Effect(index), text_bundle(f, "", 18.0, MUTED)));
        card.spawn(Node {
            column_gap: Val::Px(8.0),
            ..default()
        })
        .with_children(|row| {
            button(row, f, MenuAction::BuyPassive(index), "", 164.0);
            button(row, f, MenuAction::EquipPassive(index), "", 115.0);
        });
    });
}

fn button(
    ui: &mut ChildSpawnerCommands,
    f: &Handle<Font>,
    action: MenuAction,
    label: &str,
    width: f32,
) {
    ui.spawn((
        ShopButton,
        action,
        GlueDisabled(false),
        Button,
        Node {
            width: Val::Px(width),
            height: Val::Px(42.0),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(4.0)),
            ..default()
        },
        BackgroundColor(Color::srgb(0.23, 0.13, 0.069)),
        BorderColor::all(EDGE),
    ))
    .with_child((
        Caption::Button(action),
        centered_bundle(f, label, 19.0, GOLD),
        FocusPolicy::Pass,
    ));
}

impl Shop {
    pub(super) fn close(&mut self) {
        *self = Self::default();
    }

    pub(super) fn handle(&mut self, progress: &mut Progress, action: MenuAction) -> bool {
        match action {
            MenuAction::OpenPassives => {
                self.open = true;
                self.feedback.clear();
            }
            MenuAction::ClosePassives => self.close(),
            MenuAction::BuyPassive(index) if self.open => {
                self.feedback = progress
                    .buy(index)
                    .map_or_else(str::to_owned, |()| "Starting rank saved.".into());
            }
            MenuAction::EquipPassive(index) if self.open => {
                self.feedback = progress
                    .toggle(index)
                    .map_or_else(str::to_owned, |()| "Starting loadout saved.".into());
            }
            MenuAction::RespecPassives if self.open => {
                self.feedback = match progress.respec() {
                    Ok(refund) => format!("Refunded {refund} points. Loadout cleared."),
                    Err(error) => error.to_owned(),
                };
            }
            MenuAction::BuyPassive(_)
            | MenuAction::EquipPassive(_)
            | MenuAction::RespecPassives => {}
            _ => return self.open,
        }
        true
    }
}

#[allow(clippy::type_complexity)]
pub(super) fn refresh(
    shop: Res<Shop>,
    progress: Res<Progress>,
    mut panels: Query<(&Panel, &mut Node, &mut Visibility)>,
    mut texts: Query<(&Caption, &mut Text)>,
    mut buttons: Query<
        (
            &MenuAction,
            &Interaction,
            &mut BackgroundColor,
            &mut GlueDisabled,
        ),
        With<ShopButton>,
    >,
) {
    for (panel, mut node, mut visible) in &mut panels {
        if !matches!(panel, Panel::Passives) {
            continue;
        }
        let show = shop.open;
        let visibility = if show {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        let display = if show { Display::Flex } else { Display::None };
        if *visible != visibility {
            *visible = visibility;
        }
        if node.display != display {
            node.display = display;
        }
    }
    let spent = progress.spent_points();
    for (label, mut text) in &mut texts {
        let value = match *label {
            Caption::Balance => format!(
                "{} points   •   {}/{} equipped   •   {} spent",
                progress.points(),
                progress.equipped_count(),
                SLOTS,
                spent,
            ),
            Caption::Feedback => if progress.load_error && shop.feedback.is_empty() {
                "Could not read saved progress."
            } else {
                &shop.feedback
            }
            .to_owned(),
            Caption::Effect(index) => {
                let rank = progress.rank(index);
                let passive = &data::PASSIVES[index];
                let effect = passive_text(passive.stat, passive.per_level * f32::from(rank.max(1)));
                if rank == 0 {
                    format!("Rank 1  •  {effect}")
                } else {
                    format!("Rank {rank}  •  {effect}")
                }
            }
            Caption::Button(MenuAction::OpenPassives) => {
                format!("Passive upgrades  •  {} pts", progress.points())
            }
            Caption::Button(MenuAction::RespecPassives) => {
                if spent > 0 {
                    format!("Respec · {spent} pts")
                } else {
                    "Respec".to_owned()
                }
            }
            Caption::Button(MenuAction::BuyPassive(index)) => match progress.price(index) {
                None => "Max rank".to_owned(),
                Some(cost) if progress.rank(index) == 0 => format!("Unlock · {cost} pts"),
                Some(cost) => format!("Upgrade · {cost} pts"),
            },
            Caption::Button(MenuAction::EquipPassive(index)) => if progress.equipped(index) {
                "Unequip"
            } else if progress.rank(index) == 0 {
                "Locked"
            } else {
                "Equip"
            }
            .to_owned(),
            Caption::Button(_) => continue,
        };
        if **text != value {
            **text = value;
        }
    }
    for (action, interaction, mut background, mut disabled) in &mut buttons {
        let available = match *action {
            MenuAction::BuyPassive(index) => progress
                .price(index)
                .is_some_and(|cost| progress.points() >= cost),
            MenuAction::EquipPassive(index) => {
                progress.rank(index) > 0
                    && (progress.equipped(index) || progress.equipped_count() < SLOTS)
            }
            MenuAction::RespecPassives => spent > 0,
            _ => true,
        };
        if disabled.0 != !available {
            disabled.0 = !available;
        }
        let color = if !available {
            Color::srgb(0.14, 0.105, 0.081)
        } else if *interaction != Interaction::None {
            Color::srgb(0.38, 0.22, 0.09)
        } else {
            Color::srgb(0.23, 0.13, 0.069)
        };
        if background.0 != color {
            background.0 = color;
        }
    }
}
