//! ZMODEM 会话状态机：由挂载层的驱动线程独占驱动。

mod driver;
mod state;
mod types;

pub(crate) use driver::fsm_loop;
pub(crate) use state::Fsm;
#[cfg(test)]
pub(crate) use types::Mode;
pub(crate) use types::{FsmChannels, ZmodemCommand};
