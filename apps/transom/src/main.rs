mod controller;

use controller::Controller;
use std::sync::Arc;
use transom_core::{
    BackupService, KnownHostsStore, SecretStore, SerialPortProvider, SessionStore, SettingsStore,
};
use transom_net::SystemSerialPortProvider;
use transom_store::{
    EncryptedBackupService, JsonKnownHostsStore, JsonSessionStore, JsonSettingsStore,
    KeyringSecretStore, paths,
};

fn main() -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();

    let known_hosts: Arc<dyn KnownHostsStore> =
        Arc::new(JsonKnownHostsStore::new(paths::known_hosts_path())?);
    let session_store: Arc<dyn SessionStore> =
        Arc::new(JsonSessionStore::new(paths::sessions_path()));
    let settings_store: Arc<dyn SettingsStore> =
        Arc::new(JsonSettingsStore::new(paths::settings_path()));
    let secret_store: Arc<dyn SecretStore> = Arc::new(KeyringSecretStore::new("transom"));
    let serial_ports: Arc<dyn SerialPortProvider> = Arc::new(SystemSerialPortProvider);
    let backup: Arc<dyn BackupService> = Arc::new(EncryptedBackupService);

    let controller = Controller::start(known_hosts.clone())?;
    let services = transom_ui::UiServices {
        session_store,
        settings_store,
        secret_store,
        known_hosts,
        serial_ports,
        backup,
        command_tx: controller.command_sender(),
        event_rx: controller.event_receiver(),
    };

    transom_ui::run(services);
    drop(controller);
    Ok(())
}
