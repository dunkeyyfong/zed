//! ZedG fork: a VSCode-style activity bar, a column of icons on the far left
//! that opens panels and common views with one click.

use gpui::{
    Action, AnyElement, App, Context, Entity, FocusHandle, Hsla, IntoElement, ParentElement,
    Render, Styled, Subscription, Window, div, px,
};
use settings::{RegisterSetting, Settings, SettingsStore};
use ui::{CountBadge, IconButton, IconButtonShape, Tooltip, prelude::*};

use crate::dock::Dock;

/// Width of the activity bar, matching VSCode's compact layout.
const ACTIVITY_BAR_WIDTH: f32 = 40.;

/// Whether to show the activity bar.
#[derive(Clone, Copy, Debug, RegisterSetting)]
pub struct ActivityBarSettings {
    pub show: bool,
}

impl Settings for ActivityBarSettings {
    fn from_settings(content: &settings::SettingsContent) -> Self {
        Self {
            show: content.zedg_activity_bar.unwrap_or(true),
        }
    }
}

/// A dock panel as an activity bar button, see [`Dock::activity_bar_entries`].
pub(crate) struct PanelEntryButton {
    pub name: &'static str,
    pub icon: IconName,
    pub tooltip: &'static str,
    pub action: Box<dyn Action>,
    pub is_active: bool,
    pub badge: Option<usize>,
    pub focus_handle: FocusHandle,
}

pub struct ActivityBar {
    docks: [Entity<Dock>; 3],
    _subscriptions: Vec<Subscription>,
}

impl ActivityBar {
    /// `docks` are the left, right and bottom docks, in that order.
    pub fn new(docks: [Entity<Dock>; 3], cx: &mut Context<Self>) -> Self {
        let mut subscriptions: Vec<_> = docks
            .iter()
            .map(|dock| cx.observe(dock, |_, _, cx| cx.notify()))
            .collect();
        subscriptions.push(cx.observe_global::<SettingsStore>(|_, cx| cx.notify()));
        Self {
            docks,
            _subscriptions: subscriptions,
        }
    }

    pub fn is_visible(cx: &App) -> bool {
        ActivityBarSettings::get_global(cx).show
    }
}

impl Render for ActivityBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut panels: Vec<PanelEntryButton> = Vec::new();
        for dock in &self.docks {
            panels.extend(dock.read(cx).activity_bar_entries(window, cx));
        }
        // Views that aren't panels are opened through actions, dispatched from
        // the left dock so they reach the workspace like keyboard shortcuts do.
        let action_focus = self.docks[0].read(cx).panel_button_focus_handle(window, cx);
        let accent = cx.theme().colors().text_accent;

        let mut take_panel = |name: &str| -> Option<AnyElement> {
            let ix = panels.iter().position(|entry| entry.name == name)?;
            Some(panel_button(panels.remove(ix), accent))
        };

        // VSCode's order: Explorer, Search, Source Control, Run and Debug,
        // Extensions. Git Graph sits next to Source Control.
        let mut top: Vec<AnyElement> = Vec::new();
        top.extend(take_panel("Project Panel"));
        top.push(action_button(
            "activity-search",
            IconName::MagnifyingGlass,
            "Search",
            Box::new(crate::DeploySearch::default()),
            action_focus.clone(),
        ));
        top.extend(take_panel("GitPanel"));
        if let Ok(action) = cx.build_action("git_graph::Open", None) {
            top.push(action_button(
                "activity-git-graph",
                IconName::GitGraph,
                "Git Graph",
                action,
                action_focus.clone(),
            ));
        }
        top.extend(take_panel("DebugPanel"));
        top.push(action_button(
            "activity-extensions",
            IconName::Blocks,
            "Extensions",
            Box::new(zed_actions::Extensions::default()),
            action_focus.clone(),
        ));
        // Every other panel, in dock order (outline, terminal, agent, ...).
        top.extend(panels.into_iter().map(|entry| panel_button(entry, accent)));

        let settings = action_button(
            "activity-settings",
            IconName::Settings,
            "Settings",
            Box::new(zed_actions::OpenSettings),
            action_focus,
        );

        let colors = cx.theme().colors();
        v_flex()
            .id("activity-bar")
            .h_full()
            .w(px(ACTIVITY_BAR_WIDTH))
            .flex_none()
            .py_1()
            .justify_between()
            .bg(colors.status_bar_background)
            .border_t_1()
            .border_b_1()
            .border_r_1()
            .border_color(colors.border)
            .child(v_flex().w_full().gap_1().children(top))
            .child(v_flex().w_full().child(settings))
    }
}

fn panel_button(entry: PanelEntryButton, accent: Hsla) -> AnyElement {
    let PanelEntryButton {
        name,
        icon,
        tooltip,
        action,
        is_active,
        badge,
        focus_handle,
    } = entry;
    activity_button(
        // Include the active state in the ID so the cached tooltip refreshes.
        (name, is_active as u64),
        icon,
        tooltip,
        action,
        is_active,
        badge,
        focus_handle,
        accent,
    )
}

fn action_button(
    id: &'static str,
    icon: IconName,
    tooltip: &'static str,
    action: Box<dyn Action>,
    focus_handle: FocusHandle,
) -> AnyElement {
    // Action buttons are never shown as active, so the accent is unused.
    activity_button(
        (id, 0),
        icon,
        tooltip,
        action,
        false,
        None,
        focus_handle,
        Hsla::default(),
    )
}

fn activity_button(
    id: (&'static str, u64),
    icon: IconName,
    tooltip: &'static str,
    action: Box<dyn Action>,
    is_active: bool,
    badge: Option<usize>,
    focus_handle: FocusHandle,
    accent: Hsla,
) -> AnyElement {
    let tooltip_action = action.boxed_clone();
    let button = IconButton::new(id, icon)
        .shape(IconButtonShape::Square)
        .size(ButtonSize::Large)
        .icon_size(IconSize::Medium)
        .icon_color(Color::Muted)
        .selected_icon_color(Color::Default)
        .toggle_state(is_active)
        .aria_label(tooltip)
        .tooltip(move |_window, cx| Tooltip::for_action(tooltip, &*tooltip_action, cx))
        .on_click(move |_, window, cx| {
            window.focus(&focus_handle, cx);
            window.dispatch_action(action.boxed_clone(), cx);
        });

    h_flex()
        .relative()
        .w_full()
        .justify_center()
        // VSCode marks the active view with an accent line on the left edge.
        .when(is_active, |this| {
            this.child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .bottom_0()
                    .w(px(2.))
                    .bg(accent),
            )
        })
        .child(div().relative().child(button).when_some(
            badge.filter(|count| *count > 0),
            |this, count| this.child(CountBadge::new(count)),
        ))
        .into_any_element()
}
