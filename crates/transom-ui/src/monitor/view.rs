//! 监控抽屉容器视图与组装管道。

use crate::divider::{DividerEdge, DividerStyled};
use crate::monitor::MonitorView;
use crate::monitor::types::{SampleIntervalMode, SparklineChartType};
use crate::palette::Palette;
use crate::theme::tokens::size;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::TabId;

mod panel;
mod tab;

pub(crate) use tab::MonitorTabParams;
