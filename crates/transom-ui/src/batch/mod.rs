//! 多会话批量修改：选择模式、表单状态与弹窗渲染。

mod actions;
mod common;
mod state;
mod view;

pub(crate) use state::BatchEditForm;

#[cfg(test)]
mod tests {
    use super::state::{BatchUpdates, apply_batch_updates};
    use std::collections::HashSet;
    use transom_core::{
        AuthMethod, Connection, LocalConnection, SerialConnection, Session, SessionCatalog,
        SshConnection,
    };

    fn ssh_session(name: &str) -> Session {
        Session::new(
            name,
            Connection::Ssh(SshConnection {
                host: "example.com".into(),
                port: 22,
                username: "old".into(),
                auth: AuthMethod::Agent,
                options: Default::default(),
                proxy_jump: None,
            }),
        )
    }

    #[test]
    fn updates_only_selected_ssh_sessions() {
        let mut catalog = SessionCatalog::default();
        let ssh = ssh_session("ssh");
        let ssh_id = ssh.id;
        let local = Session::new("local", Connection::Local(LocalConnection::default()));
        let local_id = local.id;
        let serial = Session::new(
            "serial",
            Connection::Serial(SerialConnection::new("/dev/ttyS0", 9600)),
        );
        let serial_id = serial.id;
        catalog.sessions = vec![ssh, local, serial];
        let selected: HashSet<_> = [ssh_id, local_id, serial_id].into_iter().collect();

        let updates = BatchUpdates {
            port: Some(2222),
            username: Some("deploy".into()),
            auth: Some(AuthMethod::Agent),
        };
        let (updated, skipped) = apply_batch_updates(&mut catalog, &selected, &updates);
        assert_eq!((updated, skipped), (1, 2));

        let Connection::Ssh(ssh) = &catalog.sessions[0].connection else {
            panic!("应为 SSH 会话");
        };
        assert_eq!(ssh.port, 2222);
        assert_eq!(ssh.username, "deploy");
    }

    #[test]
    fn unselected_sessions_keep_their_properties() {
        let mut catalog = SessionCatalog::default();
        let selected = ssh_session("selected");
        let untouched = ssh_session("untouched");
        let selected_id = selected.id;
        catalog.sessions = vec![selected, untouched];
        let selected_ids = HashSet::from([selected_id]);

        let updates = BatchUpdates {
            port: Some(2022),
            username: None,
            auth: None,
        };
        apply_batch_updates(&mut catalog, &selected_ids, &updates);
        let Connection::Ssh(ssh) = &catalog.sessions[1].connection else {
            panic!("应为 SSH 会话");
        };
        assert_eq!(ssh.port, 22);
    }
}
