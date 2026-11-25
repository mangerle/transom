//! ZMODEM 传输事件。

use crate::ids::{ConnectionId, TabId};
use crate::protocol::zmodem::ZmodemEvent;

/// ZMODEM 传输事件。
pub enum ZmodemAppEvent {
    Zmodem {
        tab_id: TabId,
        connection_id: ConnectionId,
        event: ZmodemEvent,
    },
}
