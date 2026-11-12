//! Native wgpui desktop shell for Reef.
//!
//! The UI talks directly to the thread-affine `SessionActor`, portable
//! `Storage`, and `AppDiagnostics`. There is no webview, JavaScript bridge, or
//! Tauri command boundary in this process.

use std::any::TypeId;
use std::sync::Arc;

use reef_engine::model::DeviceIdentity;
use reef_logging::record::RecordingStatus;
use reef_session::TransportType as SessionTransport;
use reef_storage::{
    BackupSource, ChangeRecord, ConfigBackup, DeviceProfile, Storage,
    TransportType as ProfileTransport,
};
use wgpui::{
    actions, div, prelude::*, px, AnyElement, App, ClipboardItem, Context, ElementId, Entity,
    FocusHandle, FontWeight, Hsla, InteractiveElement, MouseButton, MouseDownEvent, Render,
    ScrollHandle, SharedString, WeakEntity, Window,
};

use crate::diagnostics::{
    AppDiagnostics, DiagnosticsStatus, LogFile, LogFileContent, STARTUP_CONFIG_WRITES_ENABLED,
};
use crate::operations::{
    bound_command_output, build_profile, split_name_servers, validate_device_command,
    ConnectionRequest,
};
use crate::palette;
use crate::ui;

actions!([DismissModal]);
/// A transient notification displayed in the top-right corner.
#[derive(Clone, Debug)]
struct Toast {
    id: usize,
    kind: ToastKind,
    message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ToastKind {
    Info,
    Success,
    Warning,
    Error,
}

impl ToastKind {
    fn color(&self) -> Hsla {
        match self {
            ToastKind::Info => palette::accent(),
            ToastKind::Success => palette::accent(),
            ToastKind::Warning => palette::warning(),
            ToastKind::Error => palette::danger(),
        }
    }

    fn label(&self) -> &'static str {
        match self {
            ToastKind::Info => "Info",
            ToastKind::Success => "Success",
            ToastKind::Warning => "Warning",
            ToastKind::Error => "Error",
        }
    }
}
use crate::session_actor::{
    AddVlanDraft, ConfigurationDraft, ConfigurationPreview, ConfigurationSnapshot, DeviceWorkspace,
    InterfaceDraft, RouteDraft, SessionActor,
};
use crate::text_field::TextField;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Overview,
    Interfaces,
    Vlans,
    Routing,
    Configuration,
    AccessLists,
    Dhcp,
    System,
    Nat,
    SshAccess,
    Terminal,
    Diagnostics,
    Connections,
}

impl Section {
    pub const ALL: [Section; 13] = [
        Section::Overview,
        Section::Interfaces,
        Section::Vlans,
        Section::Routing,
        Section::Configuration,
        Section::AccessLists,
        Section::Dhcp,
        Section::System,
        Section::Nat,
        Section::SshAccess,
        Section::Terminal,
        Section::Diagnostics,
        Section::Connections,
    ];

    fn label(self) -> &'static str {
        match self {
            Section::Overview => "Overview",
            Section::Interfaces => "Interfaces",
            Section::Vlans => "VLANs",
            Section::Routing => "Routing",
            Section::Configuration => "Configuration",
            Section::AccessLists => "Access Lists",
            Section::Dhcp => "DHCP",
            Section::System => "System",
            Section::Nat => "NAT",
            Section::SshAccess => "SSH & Access",
            Section::Terminal => "Terminal",
            Section::Diagnostics => "Diagnostics",
            Section::Connections => "Connections",
        }
    }

    fn subtitle(self) -> &'static str {
        match self {
            Section::Overview => "Device health, identity, and shortcuts.",
            Section::Interfaces => "Operational and configured interface state.",
            Section::Vlans => "Layer 2 broadcast domains configured on the device.",
            Section::Routing => "Static routes and next hops parsed from the device.",
            Section::Configuration => "Review, preview, and apply running-configuration changes.",
            Section::AccessLists => "Parsed access control lists.",
            Section::Dhcp => "Configured DHCP pools.",
            Section::System => "Hostname, domain, naming, and system information.",
            Section::Nat => "Network address translation configuration.",
            Section::SshAccess => "Remote access and trusted host keys.",
            Section::Terminal => "Run commands through the active Rust session.",
            Section::Diagnostics => "Portable logs, environment, and exports.",
            Section::Connections => "Saved device profiles and connection state.",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Section::Overview => "⌂",
            Section::Interfaces => "▤",
            Section::Vlans => "◇",
            Section::Routing => "⇄",
            Section::Configuration => "⌘",
            Section::AccessLists => "▦",
            Section::Dhcp => "⇥",
            Section::System => "⚙",
            Section::Nat => "⇄",
            Section::SshAccess => "⌑",
            Section::Terminal => ">_",
            Section::Diagnostics => "◉",
            Section::Connections => "⌁",
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|item| *item == self).unwrap_or(0)
    }

    fn group(self) -> NavGroup {
        match self {
            Section::Overview => NavGroup::Workspace,
            Section::Interfaces | Section::Vlans | Section::Routing => NavGroup::Operate,
            Section::Configuration
            | Section::AccessLists
            | Section::Dhcp
            | Section::System
            | Section::Nat
            | Section::SshAccess => NavGroup::Configure,
            Section::Terminal | Section::Diagnostics | Section::Connections => NavGroup::Tools,
        }
    }

    fn uses_workspace(self) -> bool {
        matches!(
            self,
            Section::Interfaces | Section::Vlans | Section::Routing
        )
    }

    fn uses_configuration(self) -> bool {
        matches!(
            self,
            Section::Configuration
                | Section::AccessLists
                | Section::Dhcp
                | Section::System
                | Section::Nat
                | Section::SshAccess
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NavGroup {
    Workspace,
    Operate,
    Configure,
    Tools,
}

impl NavGroup {
    const ALL: [NavGroup; 4] = [
        NavGroup::Workspace,
        NavGroup::Operate,
        NavGroup::Configure,
        NavGroup::Tools,
    ];

    fn label(self) -> &'static str {
        match self {
            NavGroup::Workspace => "Workspace",
            NavGroup::Operate => "Operate",
            NavGroup::Configure => "Configure",
            NavGroup::Tools => "Tools",
        }
    }

    fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|group| *group == self)
            .unwrap_or(0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TransportChoice {
    Ssh,
    Telnet,
    Serial,
}

impl TransportChoice {
    fn label(self) -> &'static str {
        match self {
            TransportChoice::Ssh => "SSH",
            TransportChoice::Telnet => "Telnet",
            TransportChoice::Serial => "Serial",
        }
    }

    fn profile(self) -> ProfileTransport {
        match self {
            TransportChoice::Ssh => ProfileTransport::Ssh,
            TransportChoice::Telnet => ProfileTransport::Telnet,
            TransportChoice::Serial => ProfileTransport::Serial,
        }
    }

    fn session(self) -> SessionTransport {
        match self {
            TransportChoice::Ssh => SessionTransport::Ssh,
            TransportChoice::Telnet => SessionTransport::Telnet,
            TransportChoice::Serial => SessionTransport::Serial,
        }
    }

    fn is_serial(self) -> bool {
        self == TransportChoice::Serial
    }
}

struct ProfileForm {
    name: Entity<TextField>,
    host: Entity<TextField>,
    port: Entity<TextField>,
    username: Entity<TextField>,
    transport: TransportChoice,
    error: Option<String>,
    saving: bool,
}

impl ProfileForm {
    fn new(cx: &mut Context<ReefApp>) -> Self {
        Self {
            name: field(cx, "Core switch", false, ""),
            host: field(cx, "192.168.1.1", false, ""),
            port: field(cx, "22", false, "22"),
            username: field(cx, "admin", false, "admin"),
            transport: TransportChoice::Ssh,
            error: None,
            saving: false,
        }
    }
}

struct ConnectionForm {
    profile: DeviceProfile,
    password: Entity<TextField>,
    enable_secret: Entity<TextField>,
    trust_host_key: bool,
    connecting: bool,
    error: Option<String>,
}

struct Connected {
    profile: DeviceProfile,
    identity: Option<DeviceIdentity>,
    identity_error: Option<String>,
    privilege_secret: Entity<TextField>,
    enabling: bool,
}

#[derive(Default)]
struct WorkspaceState {
    data: Option<DeviceWorkspace>,
    loading: bool,
    error: Option<String>,
}

struct ConfigState {
    data: Option<ConfigurationSnapshot>,
    loading: bool,
    error: Option<String>,
    preview: Option<ConfigurationPreview>,
    preview_loading: bool,
    preview_error: Option<String>,
    apply_loading: bool,
    apply_error: Option<String>,
    apply_message: Option<String>,
    backups: Vec<ConfigBackup>,
    backups_loading: bool,
    backups_error: Option<String>,
    change_history: Vec<ChangeRecord>,
    history_loading: bool,
    history_error: Option<String>,
    show_backup_panel: bool,
    backup_name: Option<Entity<TextField>>,
    known_hosts: Vec<reef_session::KnownHostEntry>,
    known_hosts_loading: bool,
    known_hosts_error: Option<String>,
    known_hosts_show_enroll: bool,
    enroll_host: Option<Entity<TextField>>,
    enroll_key_type: Option<Entity<TextField>>,
    enroll_key_hex: Option<Entity<TextField>>,
}

impl Default for ConfigState {
    fn default() -> Self {
        Self {
            data: None,
            loading: false,
            error: None,
            preview: None,
            preview_loading: false,
            preview_error: None,
            apply_loading: false,
            apply_error: None,
            apply_message: None,
            backups: Vec::new(),
            backups_loading: false,
            backups_error: None,
            change_history: Vec::new(),
            history_loading: false,
            history_error: None,
            show_backup_panel: false,
            backup_name: None,
            known_hosts: Vec::new(),
            known_hosts_loading: false,
            known_hosts_error: None,
            known_hosts_show_enroll: false,
            enroll_host: None,
            enroll_key_type: None,
            enroll_key_hex: None,
        }
    }
}

/// State for the single app-level dialog. Only one modal can be open at a
/// time; destructive confirmations replace the form dialog temporarily and
/// restore it if the user changes their mind.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Modal {
    Connection,
    Profile,
    Interface,
    Vlan,
    Route,
    SystemSettings,
    ConfirmDiscard,
    ConfirmApply,
    ConfirmRestore,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectField {
    Transport,
    InterfaceMode,
}

struct DraftForm {
    hostname: Entity<TextField>,
    domain_name: Entity<TextField>,
    name_servers: Entity<TextField>,
}

/// Edit state for adding or modifying an interface.
struct InterfaceEditForm {
    editing: bool,
    name: Entity<TextField>,
    description: Entity<TextField>,
    enabled: bool,
    mode: InterfaceModeChoice,
    access_vlan: Entity<TextField>,
    ipv4_address: Entity<TextField>,
    ipv4_prefix_len: Entity<TextField>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InterfaceModeChoice {
    Routed,
    Access,
    Trunk,
    L2,
}

impl InterfaceModeChoice {
    fn label(self) -> &'static str {
        match self {
            InterfaceModeChoice::Routed => "Routed (L3)",
            InterfaceModeChoice::Access => "Access (L2)",
            InterfaceModeChoice::Trunk => "Trunk",
            InterfaceModeChoice::L2 => "L2 (switch)",
        }
    }

    fn draft_value(self) -> &'static str {
        match self {
            InterfaceModeChoice::Routed => "routed",
            InterfaceModeChoice::Access => "access",
            InterfaceModeChoice::Trunk => "trunk",
            InterfaceModeChoice::L2 => "l2",
        }
    }

    fn from_mode(mode: &str) -> Self {
        match mode.to_ascii_lowercase().as_str() {
            "access" => InterfaceModeChoice::Access,
            "trunk" => InterfaceModeChoice::Trunk,
            "l2" => InterfaceModeChoice::L2,
            _ => InterfaceModeChoice::Routed,
        }
    }
}

/// Edit state for adding or modifying a VLAN.
struct VlanEditForm {
    editing: bool,
    id: Entity<TextField>,
    name: Entity<TextField>,
    enabled: bool,
}

/// Edit state for adding or replacing a static route.
struct RouteEditForm {
    editing: bool,
    original_prefix: Option<String>,
    prefix: Entity<TextField>,
    next_hop: Entity<TextField>,
    admin_distance: Entity<TextField>,
}

impl DraftForm {
    fn new(cx: &mut Context<ReefApp>) -> Self {
        Self {
            hostname: field(cx, "Router-01", false, ""),
            domain_name: field(cx, "example.net", false, ""),
            name_servers: field(cx, "1.1.1.1, 8.8.8.8", false, ""),
        }
    }
}

struct TerminalState {
    output: String,
    command: Entity<TextField>,
    busy: bool,
    message: Option<String>,
    error: Option<String>,
    recording: Option<RecordingStatus>,
    recording_busy: bool,
    scroll: ScrollHandle,
    command_history: Vec<String>,
    history_index: usize,
}

impl TerminalState {
    fn new(cx: &mut Context<ReefApp>) -> Self {
        Self {
            output: "Reef device terminal\nCommands are executed through the active Rust session.\nBackend recording captures full command output even when this view truncates it.\n\n$ "
                .to_string(),
            command: field(cx, "show version", false, ""),
            busy: false,
            message: None,
            error: None,
            recording: None,
            recording_busy: false,
            scroll: ScrollHandle::new(),
            command_history: Vec::new(),
            history_index: 0,
        }
    }

    fn reset(&mut self) {
        self.output.clear();
        self.output.push_str("$ ");
        self.busy = false;
        self.message = None;
        self.error = None;
        self.recording = None;
    }
}

struct DiagnosticsState {
    status: Option<DiagnosticsStatus>,
    loading: bool,
    error: Option<String>,
    files: Vec<LogFile>,
    files_loading: bool,
    files_error: Option<String>,
    selected: Option<LogFileContent>,
    view_error: Option<String>,
    message: Option<String>,
    busy: bool,
    note: Entity<TextField>,
}

impl DiagnosticsState {
    fn new(cx: &mut Context<ReefApp>) -> Self {
        Self {
            status: None,
            loading: false,
            error: None,
            files: Vec::new(),
            files_loading: false,
            files_error: None,
            selected: None,
            view_error: None,
            message: None,
            busy: false,
            note: field(
                cx,
                "What did you test? What failed? Include device model and transport.",
                false,
                "",
            ),
        }
    }
}

pub struct ReefApp {
    session: SessionActor,
    storage: Arc<Storage>,
    diagnostics: Arc<AppDiagnostics>,
    active: Section,
    content_scroll: ScrollHandle,
    sidebar_collapsed: bool,
    collapsed_groups: [bool; 4],
    profiles: Vec<DeviceProfile>,
    profiles_loading: bool,
    profile_error: Option<String>,
    profile_form: ProfileForm,
    connection: Option<ConnectionForm>,
    connected: Option<Connected>,
    workspace: WorkspaceState,
    config: ConfigState,
    draft: DraftForm,
    interface_edit: Option<InterfaceEditForm>,
    vlan_edit: Option<VlanEditForm>,
    route_edit: Option<RouteEditForm>,
    modal: Option<Modal>,
    pending_modal: Option<Modal>,
    modal_error: Option<String>,
    open_select: Option<SelectField>,
    modal_focus: FocusHandle,
    escape_bound: bool,
    pending_restore: Option<String>,
    show_details: bool,
    terminal: TerminalState,
    diagnostics_ui: DiagnosticsState,
    toasts: Vec<Toast>,
    next_toast_id: usize,
}

impl ReefApp {
    pub fn new(
        session: SessionActor,
        storage: Arc<Storage>,
        diagnostics: Arc<AppDiagnostics>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut app = Self {
            session,
            storage,
            diagnostics,
            active: Section::Overview,
            content_scroll: ScrollHandle::new(),
            sidebar_collapsed: false,
            collapsed_groups: [false; 4],
            profiles: Vec::new(),
            profiles_loading: false,
            profile_error: None,
            profile_form: ProfileForm::new(cx),
            connection: None,
            connected: None,
            workspace: WorkspaceState::default(),
            config: ConfigState::default(),
            draft: DraftForm::new(cx),
            interface_edit: None,
            vlan_edit: None,
            route_edit: None,
            modal: None,
            pending_modal: None,
            modal_error: None,
            open_select: None,
            modal_focus: cx.focus_handle(),
            escape_bound: false,
            pending_restore: None,
            show_details: false,
            terminal: TerminalState::new(cx),
            diagnostics_ui: DiagnosticsState::new(cx),
            toasts: Vec::new(),
            next_toast_id: 0,
        };
        app.config.backup_name = Some(field(cx, "Backup name", false, ""));
        app.config.enroll_host = Some(field(cx, "Host", false, ""));
        app.config.enroll_key_type = Some(field(cx, "Key type (e.g. ssh-rsa)", false, ""));
        app.config.enroll_key_hex = Some(field(cx, "Key hex", false, ""));
        app.install_submit_handlers(cx);
        app
    }

    /// Called once after the entity is registered so background loading can
    /// safely upgrade its weak handle.
    pub fn bootstrap(&mut self, cx: &mut Context<Self>) {
        self.refresh_profiles(cx);
    }

    fn install_submit_handlers(&mut self, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        set_submit(
            weak.clone(),
            &self.profile_form.name,
            cx,
            ReefApp::save_profile,
        );
        set_submit(
            weak.clone(),
            &self.profile_form.host,
            cx,
            ReefApp::save_profile,
        );
        set_submit(
            weak.clone(),
            &self.profile_form.port,
            cx,
            ReefApp::save_profile,
        );
        set_submit(
            weak.clone(),
            &self.profile_form.username,
            cx,
            ReefApp::save_profile,
        );
        set_submit(
            weak.clone(),
            &self.diagnostics_ui.note,
            cx,
            ReefApp::save_test_note,
        );
        set_submit_with_text(
            weak,
            &self.terminal.command,
            cx,
            ReefApp::submit_terminal_command,
        );
    }

    /// Show a transient notification toast.
    fn show_toast(&mut self, kind: ToastKind, message: impl Into<String>) {
        let id = self.next_toast_id;
        self.next_toast_id += 1;
        self.toasts.push(Toast {
            id,
            kind,
            message: message.into(),
        });
        // Keep only the most recent 5 toasts.
        if self.toasts.len() > 5 {
            self.toasts.remove(0);
        }
    }

    /// Dismiss a toast by id.
    fn dismiss_toast(&mut self, id: usize) {
        self.toasts.retain(|t| t.id != id);
    }

    fn install_connection_handlers(
        &mut self,
        form: &Entity<ConnectionForm>,
        cx: &mut Context<Self>,
    ) {
        let weak = cx.entity().downgrade();
        let (password, enable_secret) = {
            let form = form.read(cx);
            (form.password.clone(), form.enable_secret.clone())
        };
        for field in [password, enable_secret] {
            let weak = weak.clone();
            field.update(cx, |field, _| {
                let weak = weak.clone();
                field.set_on_submit(move |_text, _window, cx| {
                    let weak = weak.clone();
                    cx.defer(move |cx| {
                        weak.update(cx, |this, cx| this.connect_selected(cx)).ok();
                    });
                });
            });
        }
    }

    fn install_connected_handlers(
        &mut self,
        connected: &Entity<Connected>,
        cx: &mut Context<Self>,
    ) {
        let weak = cx.entity().downgrade();
        let field = connected.read(cx).privilege_secret.clone();
        field.update(cx, |field, _| {
            let weak = weak.clone();
            field.set_on_submit(move |_text, _window, cx| {
                let weak = weak.clone();
                cx.defer(move |cx| {
                    weak.update(cx, |this, cx| this.enable_and_identify(cx))
                        .ok();
                });
            });
        });
    }

    fn navigate(&mut self, section: Section, cx: &mut Context<Self>) {
        self.active = section;
        self.close_modal();
        if section.uses_workspace() {
            self.refresh_workspace(cx);
        } else if section.uses_configuration() {
            self.refresh_configuration(cx);
            if section == Section::Configuration {
                self.refresh_backups(cx);
                self.refresh_change_history(cx);
            }
        } else if section == Section::Overview {
            if self.connected.is_some() {
                self.refresh_workspace(cx);
                if self.config.data.is_none() {
                    self.refresh_configuration(cx);
                }
            }
        } else if section == Section::Diagnostics {
            self.refresh_diagnostics(cx);
            self.refresh_log_files(cx);
        } else if section == Section::Terminal {
            self.refresh_recording(cx);
        } else if section == Section::SshAccess {
            self.refresh_known_hosts(cx);
        }
        cx.notify();
    }

    // ------------------------------------------------------------------
    // Modal lifecycle
    // ------------------------------------------------------------------

    fn open_modal(&mut self, modal: Modal, cx: &mut Context<Self>) {
        self.modal = Some(modal);
        self.pending_modal = None;
        self.modal_error = None;
        self.open_select = None;
        cx.notify();
    }

    fn close_modal(&mut self) {
        self.modal = None;
        self.pending_modal = None;
        self.modal_error = None;
        self.open_select = None;
    }

    /// Close the current dialog unless it contains unsaved input. In that case
    /// replace it with a confirmation dialog that can restore the form.
    fn request_modal_close(&mut self, cx: &mut Context<Self>) {
        if self.modal_is_dirty(cx) {
            if let Some(modal) = self.modal.take() {
                self.pending_modal = Some(modal);
            }
            self.modal = Some(Modal::ConfirmDiscard);
            self.open_select = None;
            self.modal_error = None;
        } else {
            self.close_modal();
        }
        cx.notify();
    }

    fn discard_modal(&mut self, cx: &mut Context<Self>) {
        self.close_modal();
        self.connection = None;
        self.interface_edit = None;
        self.vlan_edit = None;
        self.route_edit = None;
        cx.notify();
    }

    fn resume_modal(&mut self, cx: &mut Context<Self>) {
        if let Some(modal) = self.pending_modal.take() {
            self.modal = Some(modal);
        } else {
            self.modal = None;
        }
        cx.notify();
    }

    fn modal_is_dirty(&self, cx: &Context<Self>) -> bool {
        match self.modal {
            Some(Modal::Connection) => self.connection.as_ref().is_some_and(|form| {
                form.trust_host_key
                    || !form.password.read(cx).is_empty()
                    || !form.enable_secret.read(cx).is_empty()
            }),
            Some(Modal::Profile) => {
                !self.profile_form.name.read(cx).is_empty()
                    || !self.profile_form.host.read(cx).is_empty()
                    || !self.profile_form.username.read(cx).is_empty()
            }
            Some(Modal::Interface) => self.interface_edit.as_ref().is_some_and(|form| {
                !form.name.read(cx).is_empty()
                    || !form.description.read(cx).is_empty()
                    || !form.ipv4_address.read(cx).is_empty()
            }),
            Some(Modal::Vlan) => self
                .vlan_edit
                .as_ref()
                .is_some_and(|form| !form.id.read(cx).is_empty() || !form.name.read(cx).is_empty()),
            Some(Modal::Route) => self.route_edit.as_ref().is_some_and(|form| {
                !form.prefix.read(cx).is_empty() || !form.next_hop.read(cx).is_empty()
            }),
            Some(Modal::SystemSettings) => {
                !self.draft.hostname.read(cx).is_empty()
                    || !self.draft.domain_name.read(cx).is_empty()
                    || !self.draft.name_servers.read(cx).is_empty()
            }
            _ => false,
        }
    }

    fn dismiss_modal(&mut self, cx: &mut Context<Self>) {
        match self.modal.clone() {
            None => {}
            Some(Modal::ConfirmDiscard) => self.resume_modal(cx),
            Some(Modal::ConfirmApply) | Some(Modal::ConfirmRestore) => {
                self.pending_restore = None;
                self.close_modal();
                cx.notify();
            }
            Some(Modal::Connection)
                if self.connection.as_ref().is_some_and(|form| form.connecting) => {}
            Some(_) => self.request_modal_close(cx),
        }
    }

    fn open_interface_modal(
        &mut self,
        interface: Option<&crate::session_actor::ConfiguredInterface>,
        cx: &mut Context<Self>,
    ) {
        let (editing, name, description, enabled, mode, access_vlan, address, prefix) =
            match interface {
                Some(interface) => {
                    let (address, prefix) = interface
                        .ipv4
                        .as_deref()
                        .and_then(|value| value.split_once('/'))
                        .map(|(address, prefix)| (address.to_string(), prefix.to_string()))
                        .unwrap_or_default();
                    (
                        true,
                        interface.name.clone(),
                        interface.description.clone().unwrap_or_default(),
                        interface.enabled,
                        InterfaceModeChoice::from_mode(&interface.mode),
                        interface
                            .access_vlan
                            .map(|vlan| vlan.to_string())
                            .unwrap_or_default(),
                        address,
                        prefix,
                    )
                }
                None => (
                    false,
                    String::new(),
                    String::new(),
                    true,
                    InterfaceModeChoice::Routed,
                    String::new(),
                    String::new(),
                    String::new(),
                ),
            };

        self.interface_edit = Some(InterfaceEditForm {
            editing,
            name: field_owned(cx, "GigabitEthernet1/0/1", false, &name),
            description: field_owned(cx, "Interface description", false, &description),
            enabled,
            mode,
            access_vlan: field_owned(cx, "10", false, &access_vlan),
            ipv4_address: field_owned(cx, "10.0.0.1", false, &address),
            ipv4_prefix_len: field_owned(cx, "24", false, &prefix),
        });
        self.open_modal(Modal::Interface, cx);
    }

    fn open_vlan_modal(
        &mut self,
        vlan: Option<&reef_engine::model::VlanConfig>,
        cx: &mut Context<Self>,
    ) {
        let (editing, id, name, enabled) = match vlan {
            Some(vlan) => (true, vlan.id.to_string(), vlan.name.clone(), vlan.enabled),
            None => (false, String::new(), String::new(), true),
        };
        self.vlan_edit = Some(VlanEditForm {
            editing,
            id: field_owned(cx, "10", false, &id),
            name: field_owned(cx, "USERS", false, &name),
            enabled,
        });
        self.open_modal(Modal::Vlan, cx);
    }

    fn open_route_modal(
        &mut self,
        route: Option<&reef_engine::model::StaticRoute>,
        cx: &mut Context<Self>,
    ) {
        let (editing, prefix, next_hop, admin_distance) = match route {
            Some(route) => (
                true,
                route.prefix.to_string(),
                route.next_hop.to_string(),
                route
                    .admin_distance
                    .map(|distance| distance.to_string())
                    .unwrap_or_default(),
            ),
            None => (false, String::new(), String::new(), String::new()),
        };
        self.route_edit = Some(RouteEditForm {
            editing,
            original_prefix: route.map(|route| route.prefix.to_string()),
            prefix: field_owned(cx, "192.0.2.0/24", false, &prefix),
            next_hop: field_owned(cx, "10.0.0.1", false, &next_hop),
            admin_distance: field_owned(cx, "1", false, &admin_distance),
        });
        self.open_modal(Modal::Route, cx);
    }

    // ------------------------------------------------------------------
    // Profiles and connections
    // ------------------------------------------------------------------

    fn refresh_profiles(&mut self, cx: &mut Context<Self>) {
        if self.profiles_loading {
            return;
        }
        self.profiles_loading = true;
        self.profile_error = None;
        cx.notify();

        let storage = Arc::clone(&self.storage);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    storage.list_profiles().map_err(|error| error.to_string())
                })
                .await;
            this.update(cx, |this, cx| {
                this.profiles_loading = false;
                match result {
                    Ok(profiles) => {
                        this.profiles = profiles;
                        this.profile_error = None;
                    }
                    Err(error) => this.profile_error = Some(error),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn select_transport(&mut self, transport: TransportChoice, cx: &mut Context<Self>) {
        self.profile_form.transport = transport;
        let current = self.profile_form.port.read(cx).text();
        if !transport.is_serial() {
            let default_port = if transport == TransportChoice::Telnet {
                "23"
            } else {
                "22"
            };
            if current.is_empty() || current == "22" || current == "23" {
                self.profile_form
                    .port
                    .update(cx, |field, cx| field.set_content(default_port, cx));
            }
        }
        cx.notify();
    }

    fn save_profile(&mut self, cx: &mut Context<Self>) {
        if self.profile_form.saving {
            return;
        }
        let name = self.profile_form.name.read(cx).text();
        let host = self.profile_form.host.read(cx).text();
        let port = self.profile_form.port.read(cx).text();
        let username = self.profile_form.username.read(cx).text();
        let transport = self.profile_form.transport;

        let profile = match build_profile(&name, &host, &port, transport.profile(), &username) {
            Ok(profile) => profile,
            Err(error) => {
                self.profile_form.error = Some(error);
                cx.notify();
                return;
            }
        };

        self.profile_form.saving = true;
        self.profile_form.error = None;
        cx.notify();

        let storage = Arc::clone(&self.storage);
        let saved_profile = profile.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    storage
                        .save_profile(&saved_profile)
                        .map_err(|error| error.to_string())
                })
                .await;
            this.update(cx, |this, cx| {
                this.profile_form.saving = false;
                match result {
                    Ok(()) => {
                        this.profiles.push(profile);
                        this.profiles.sort_by(|a, b| a.name.cmp(&b.name));
                        this.profile_form.error = None;
                        this.profile_form
                            .name
                            .update(cx, |field, cx| field.clear(cx));
                        this.profile_form
                            .host
                            .update(cx, |field, cx| field.clear(cx));
                        let reset_port = if this.profile_form.transport == TransportChoice::Telnet {
                            "23"
                        } else {
                            "22"
                        };
                        this.profile_form
                            .port
                            .update(cx, |field, cx| field.set_content(reset_port, cx));
                        this.show_toast(ToastKind::Success, "Profile saved");
                        this.close_modal();
                    }
                    Err(error) => this.profile_form.error = Some(error),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn delete_profile(&mut self, profile: DeviceProfile, cx: &mut Context<Self>) {
        let storage = Arc::clone(&self.storage);
        let id = profile.id.clone();
        self.profile_error = None;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    storage
                        .delete_profile(&id)
                        .map_err(|error| error.to_string())
                })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(()) => {
                        this.profiles.retain(|item| item.id != profile.id);
                        if this
                            .connection
                            .as_ref()
                            .is_some_and(|form| form.profile.id == profile.id)
                        {
                            this.connection = None;
                        }
                        this.show_toast(ToastKind::Success, "Profile deleted");
                    }
                    Err(error) => this.profile_error = Some(error),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn select_profile(&mut self, profile: DeviceProfile, cx: &mut Context<Self>) {
        let form = cx.new(|cx| ConnectionForm {
            profile,
            password: field(cx, "Device login password", true, ""),
            enable_secret: field(cx, "Privilege escalation secret", true, ""),
            trust_host_key: false,
            connecting: false,
            error: None,
        });
        self.install_connection_handlers(&form, cx);
        self.connection = Some(form.read(cx).clone_for_state());
        // Keep the entity alive by re-reading it from the app; the connection
        // form state itself is plain data plus field entities.
        let _ = form;
        self.open_modal(Modal::Connection, cx);
    }

    fn connect_selected(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.connection.as_mut() else {
            return;
        };
        if form.connecting {
            return;
        }
        let profile = form.profile.clone();
        let password = form.password.update(cx, |field, cx| field.take_content(cx));
        let enable_secret = form
            .enable_secret
            .update(cx, |field, cx| field.take_content(cx));
        let trust = form.trust_host_key;
        form.error = None;
        form.connecting = true;

        let request = ConnectionRequest {
            device_id: profile.id.clone(),
            host: profile.host.clone(),
            port: profile.port,
            transport: profile.transport.clone().into_session(),
            username: profile.username.clone(),
            password: non_empty(password),
            enable_secret: non_empty(enable_secret.clone()),
            serial_port: (profile.transport == ProfileTransport::Serial)
                .then(|| profile.host.clone()),
            serial_baud: Some(9600),
            accept_unknown_host_key: trust,
        };
        let config = match request.into_config() {
            Ok(config) => config,
            Err(error) => {
                form.connecting = false;
                form.error = Some(error);
                cx.notify();
                return;
            }
        };

        let session = self.session.clone();
        let previous = self.connected.as_ref().and_then(|connected| {
            (connected.profile.id != profile.id).then(|| connected.profile.id.clone())
        });
        let enable_secret = non_empty(enable_secret);
        cx.notify();

        cx.spawn(async move |this, cx| {
            if let Some(previous) = previous {
                if let Err(error) = session.disconnect(previous).await {
                    tracing::warn!(%error, "failed to disconnect previous device");
                }
            }

            if let Err(error) = session.connect(profile.id.clone(), config).await {
                this.update(cx, |this, cx| {
                    if let Some(form) = this.connection.as_mut() {
                        form.connecting = false;
                        form.error = Some(error);
                    }
                    cx.notify();
                })
                .ok();
                return;
            }

            let enable_error = match enable_secret {
                Some(secret) => session.enable(profile.id.clone(), secret).await.err(),
                None => None,
            };
            let identity_result = session.identify(profile.id.clone()).await;
            let recording = session
                .recording_status(profile.id.clone())
                .await
                .ok()
                .flatten();

            this.update(cx, |this, cx| {
                let identity = identity_result.as_ref().ok().cloned();
                let identity_error =
                    enable_error.or_else(|| identity_result.as_ref().err().cloned());
                let connected = cx.new(|cx| Connected {
                    profile,
                    identity,
                    identity_error,
                    privilege_secret: field(cx, "Privilege escalation secret", true, ""),
                    enabling: false,
                });
                this.install_connected_handlers(&connected, cx);
                this.connected = Some(connected.read(cx).clone_for_state());
                this.connection = None;
                this.close_modal();
                this.workspace = WorkspaceState::default();
                this.config = ConfigState::default();
                this.terminal.reset();
                this.terminal.recording = recording;
                this.active = Section::Overview;
                this.show_toast(ToastKind::Success, "Connected to device");
                this.refresh_workspace(cx);
                this.refresh_configuration(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn enable_and_identify(&mut self, cx: &mut Context<Self>) {
        let Some(connected) = self.connected.as_mut() else {
            return;
        };
        if connected.enabling {
            return;
        }
        let secret = connected
            .privilege_secret
            .update(cx, |field, cx| field.take_content(cx));
        if secret.trim().is_empty() {
            connected.identity_error = Some("enable secret cannot be empty".to_string());
            cx.notify();
            return;
        }
        connected.enabling = true;
        connected.identity_error = None;
        let session = self.session.clone();
        let device_id = connected.profile.id.clone();
        cx.notify();

        cx.spawn(async move |this, cx| {
            let enable_result = session.enable(device_id.clone(), secret).await;
            let identity_result = if enable_result.is_ok() {
                Some(session.identify(device_id).await)
            } else {
                None
            };
            this.update(cx, |this, cx| {
                if let Some(connected) = this.connected.as_mut() {
                    connected.enabling = false;
                    match (enable_result, identity_result) {
                        (Err(error), _) => connected.identity_error = Some(error),
                        (Ok(()), Some(Ok(identity))) => {
                            connected.identity = Some(identity);
                            connected.identity_error = None;
                        }
                        (Ok(()), Some(Err(error))) => connected.identity_error = Some(error),
                        (Ok(()), None) => {}
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn disconnect(&mut self, cx: &mut Context<Self>) {
        let Some(connected) = self.connected.as_ref() else {
            return;
        };
        let session = self.session.clone();
        let device_id = connected.profile.id.clone();
        cx.spawn(async move |this, cx| {
            let result = session.disconnect(device_id).await;
            this.update(cx, |this, cx| {
                this.connected = None;
                this.workspace = WorkspaceState::default();
                this.config = ConfigState::default();
                this.terminal.reset();
                if let Err(error) = result {
                    tracing::warn!(%error, "device disconnect completed with an error");
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    // ------------------------------------------------------------------
    // Device workspace / configuration
    // ------------------------------------------------------------------

    fn refresh_workspace(&mut self, cx: &mut Context<Self>) {
        let Some(connected) = self.connected.as_ref() else {
            return;
        };
        if self.workspace.loading {
            return;
        }
        let device_id = connected.profile.id.clone();
        let session = self.session.clone();
        self.workspace.loading = true;
        self.workspace.error = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = session.inspect(device_id).await;
            this.update(cx, |this, cx| {
                this.workspace.loading = false;
                match result {
                    Ok(data) => {
                        this.workspace.data = Some(data);
                        this.workspace.error = None;
                    }
                    Err(error) => this.workspace.error = Some(error),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn refresh_configuration(&mut self, cx: &mut Context<Self>) {
        let Some(connected) = self.connected.as_ref() else {
            return;
        };
        if self.config.loading {
            return;
        }
        let device_id = connected.profile.id.clone();
        let session = self.session.clone();
        self.config.loading = true;
        self.config.error = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = session.get_configuration(device_id).await;
            this.update(cx, |this, cx| {
                this.config.loading = false;
                match result {
                    Ok(data) => {
                        this.config.data = Some(data);
                        this.config.error = None;
                    }
                    Err(error) => this.config.error = Some(error),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn preview_configuration(&mut self, cx: &mut Context<Self>) {
        let Some(connected) = self.connected.as_ref() else {
            return;
        };
        if self.config.preview_loading {
            return;
        }
        let hostname = self.draft.hostname.read(cx).text().trim().to_string();
        let domain_name = self.draft.domain_name.read(cx).text().trim().to_string();
        let name_servers = split_name_servers(&self.draft.name_servers.read(cx).text());
        if hostname.is_empty() && domain_name.is_empty() && name_servers.is_empty() {
            self.modal_error =
                Some("Set at least one system field before generating a preview.".to_string());
            cx.notify();
            return;
        }

        let draft = ConfigurationDraft {
            hostname: non_empty(hostname),
            domain_name: non_empty(domain_name),
            add_name_servers: name_servers,
            remove_name_servers: Vec::new(),
            add_vlans: Vec::new(),
            remove_vlan_ids: Vec::new(),
            add_interfaces: Vec::new(),
            remove_interface_names: Vec::new(),
            add_routes: Vec::new(),
            remove_route_prefixes: Vec::new(),
        };
        let device_id = connected.profile.id.clone();
        let session = self.session.clone();
        self.config.preview_loading = true;
        self.config.preview_error = None;
        self.config.preview = None;
        self.modal_error = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = session.preview_configuration(device_id, draft).await;
            this.update(cx, |this, cx| {
                this.config.preview_loading = false;
                match result {
                    Ok(preview) => {
                        let has_errors = !preview.errors.is_empty();
                        this.config.preview = Some(preview);
                        this.config.preview_error = None;
                        this.close_modal();
                        this.active = Section::Configuration;
                        if has_errors {
                            this.show_toast(
                                ToastKind::Warning,
                                "Preview generated with validation errors",
                            );
                        } else {
                            this.show_toast(ToastKind::Success, "Preview generated");
                        }
                    }
                    Err(error) => {
                        this.config.preview_error = Some(error.clone());
                        this.modal_error = Some(error.clone());
                        this.show_toast(ToastKind::Error, format!("Preview failed: {error}"));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn apply_configuration(&mut self, cx: &mut Context<Self>) {
        let Some(connected) = self.connected.as_ref() else {
            return;
        };
        if self.config.apply_loading {
            return;
        }
        let Some(preview) = self.config.preview.clone() else {
            self.config.apply_error = Some("generate a preview before applying".to_string());
            cx.notify();
            return;
        };
        if preview.commands.is_empty() {
            self.config.apply_error = Some("no commands to apply".to_string());
            cx.notify();
            return;
        }
        if !preview.errors.is_empty() {
            self.config.apply_error =
                Some("resolve preview validation errors before applying".to_string());
            cx.notify();
            return;
        }
        if preview.affects_connectivity {
            self.config.apply_error = Some(
                "this change may affect management connectivity and cannot be applied".to_string(),
            );
            cx.notify();
            return;
        }

        let commands = preview.commands.clone();
        let device_id = connected.profile.id.clone();
        let session = self.session.clone();
        self.config.apply_loading = true;
        self.config.apply_error = None;
        self.config.apply_message = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = session.apply_configuration(device_id, commands).await;
            this.update(cx, |this, cx| {
                this.config.apply_loading = false;
                match result {
                    Ok(_output) => {
                        this.config.apply_message =
                            Some("configuration applied successfully".to_string());
                        this.config.preview = None;
                        this.config.apply_error = None;
                        this.close_modal();
                        this.show_toast(ToastKind::Success, "Configuration applied");
                        this.refresh_configuration(cx);
                        this.refresh_workspace(cx);
                        this.refresh_change_history(cx);
                    }
                    Err(error) => {
                        this.config.apply_error = Some(error.clone());
                        this.show_toast(ToastKind::Error, format!("Apply failed: {error}"));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn backup_configuration(&mut self, cx: &mut Context<Self>) {
        let Some(connected) = self.connected.as_ref() else {
            return;
        };
        if self.config.backups_loading {
            return;
        }
        let device_id = connected.profile.id.clone();
        let device_name = connected.profile.name.clone();
        let storage = Arc::clone(&self.storage);
        let session = self.session.clone();
        self.config.backups_loading = true;
        self.config.backups_error = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            // Get raw running config
            let config_result = session
                .send_command(device_id.clone(), "show running-config".to_string())
                .await;
            match config_result {
                Ok(running_config) => {
                    let backup = ConfigBackup {
                        id: format!(
                            "backup-{}-{}",
                            device_id,
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs()
                        ),
                        device_id,
                        device_name,
                        created_at: chrono::Utc::now().to_rfc3339(),
                        running_config,
                        source: BackupSource::Manual,
                    };
                    let save_result = storage.save_backup(&backup);
                    this.update(cx, |this, cx| {
                        this.config.backups_loading = false;
                        match save_result {
                            Ok(()) => {
                                this.config.backups.insert(0, backup.clone());
                                this.config.show_backup_panel = true;
                                this.show_toast(ToastKind::Success, "Configuration backed up");
                            }
                            Err(error) => {
                                this.config.backups_error = Some(error.to_string());
                                this.show_toast(
                                    ToastKind::Error,
                                    format!("Backup failed: {error}"),
                                );
                            }
                        }
                        cx.notify();
                    });
                }
                Err(error) => {
                    this.update(cx, |this, cx| {
                        this.config.backups_loading = false;
                        this.config.backups_error = Some(error.clone());
                        this.show_toast(ToastKind::Error, format!("Backup failed: {error}"));
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    fn restore_configuration(&mut self, backup_id: String, cx: &mut Context<Self>) {
        let Some(connected) = self.connected.as_ref() else {
            return;
        };
        if self.config.apply_loading {
            return;
        }
        let backup = match self.config.backups.iter().find(|b| b.id == backup_id) {
            Some(b) => b.clone(),
            None => {
                self.config.apply_error = Some("backup not found".to_string());
                cx.notify();
                return;
            }
        };
        let device_id = connected.profile.id.clone();
        let device_name = connected.profile.name.clone();
        let storage = Arc::clone(&self.storage);
        let session = self.session.clone();
        self.config.apply_loading = true;
        self.config.apply_error = None;
        cx.notify();

        let device_id_for_record = device_id.clone();
        let device_name_for_record = device_name.clone();

        cx.spawn(async move |this, cx| {
            // Split config into commands
            let commands: Vec<String> = backup
                .running_config
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| l.trim().to_string())
                .collect();
            let result = session.apply_configuration(device_id, commands).await;
            this.update(cx, |this, cx| {
                this.config.apply_loading = false;
                match result {
                    Ok(_output) => {
                        // Record the change
                        let record = ChangeRecord {
                            id: format!(
                                "change-{}-{}",
                                device_id_for_record,
                                std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_secs()
                            ),
                            device_id: device_id_for_record,
                            device_name: device_name_for_record,
                            created_at: chrono::Utc::now().to_rfc3339(),
                            diff: format!("Restored from backup {}", backup.id),
                            verification: "Applied via restore".to_string(),
                            success: true,
                            rolled_back: false,
                        };
                        let _ = storage.record_change(&record);
                        this.config.apply_message =
                            Some("configuration restored from backup".to_string());
                        this.config.apply_error = None;
                        this.show_toast(ToastKind::Success, "Configuration restored");
                    }
                    Err(error) => {
                        this.config.apply_error = Some(error.clone());
                        this.show_toast(ToastKind::Error, format!("Restore failed: {error}"));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn refresh_backups(&mut self, cx: &mut Context<Self>) {
        let Some(connected) = self.connected.as_ref() else {
            return;
        };
        if self.config.backups_loading {
            return;
        }
        let device_id = connected.profile.id.clone();
        let storage = Arc::clone(&self.storage);
        self.config.backups_loading = true;
        self.config.backups_error = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = storage.list_backups(&device_id);
            this.update(cx, |this, cx| {
                this.config.backups_loading = false;
                match result {
                    Ok(backups) => {
                        this.config.backups = backups;
                        this.config.backups_error = None;
                    }
                    Err(error) => {
                        this.config.backups_error = Some(error.to_string());
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn refresh_change_history(&mut self, cx: &mut Context<Self>) {
        let Some(connected) = self.connected.as_ref() else {
            return;
        };
        if self.config.history_loading {
            return;
        }
        let device_id = connected.profile.id.clone();
        let storage = Arc::clone(&self.storage);
        self.config.history_loading = true;
        self.config.history_error = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = storage.list_history(&device_id);
            this.update(cx, |this, cx| {
                this.config.history_loading = false;
                match result {
                    Ok(history) => {
                        this.config.change_history = history;
                        this.config.history_error = None;
                    }
                    Err(error) => {
                        this.config.history_error = Some(error.to_string());
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn refresh_known_hosts(&mut self, cx: &mut Context<Self>) {
        if self.config.known_hosts_loading {
            return;
        }
        let session = self.session.clone();
        self.config.known_hosts_loading = true;
        self.config.known_hosts_error = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = session.list_known_hosts().await;
            this.update(cx, |this, cx| {
                this.config.known_hosts_loading = false;
                match result {
                    Ok(hosts) => {
                        this.config.known_hosts = hosts;
                        this.config.known_hosts_error = None;
                    }
                    Err(error) => {
                        this.config.known_hosts_error = Some(error);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn enroll_host_key_submit(&mut self, cx: &mut Context<Self>) {
        if self.config.known_hosts_loading {
            return;
        }
        let host = self
            .config
            .enroll_host
            .as_ref()
            .unwrap()
            .read(cx)
            .text()
            .trim()
            .to_string();
        let key_type = self
            .config
            .enroll_key_type
            .as_ref()
            .unwrap()
            .read(cx)
            .text()
            .trim()
            .to_string();
        let key_hex = self
            .config
            .enroll_key_hex
            .as_ref()
            .unwrap()
            .read(cx)
            .text()
            .trim()
            .to_string();
        if host.is_empty() || key_type.is_empty() || key_hex.is_empty() {
            self.config.known_hosts_error =
                Some("Host, key type, and key hex are required".to_string());
            cx.notify();
            return;
        }
        let session = self.session.clone();
        self.config.known_hosts_loading = true;
        self.config.known_hosts_error = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = session
                .enroll_host_key(host.clone(), key_type.clone(), key_hex.clone())
                .await;
            this.update(cx, |this, cx| {
                this.config.known_hosts_loading = false;
                match result {
                    Ok(()) => {
                        this.config.known_hosts_show_enroll = false;
                        this.config
                            .enroll_host
                            .as_mut()
                            .unwrap()
                            .update(cx, |f, cx| f.clear(cx));
                        this.config
                            .enroll_key_type
                            .as_mut()
                            .unwrap()
                            .update(cx, |f, cx| f.clear(cx));
                        this.config
                            .enroll_key_hex
                            .as_mut()
                            .unwrap()
                            .update(cx, |f, cx| f.clear(cx));
                        this.show_toast(
                            ToastKind::Success,
                            format!("Host key enrolled for {host}"),
                        );
                        // Refresh known hosts list
                        let session2 = this.session.clone();
                        cx.spawn(async move |this2, cx2| {
                            let _ = session2.list_known_hosts().await;
                            let _ = this2;
                            let _ = cx2;
                        })
                        .detach();
                    }
                    Err(error) => {
                        this.config.known_hosts_error = Some(error);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn remove_known_host(&mut self, host: String, cx: &mut Context<Self>) {
        if self.config.known_hosts_loading {
            return;
        }
        let session = self.session.clone();
        self.config.known_hosts_loading = true;
        self.config.known_hosts_error = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = session.remove_known_host(host.clone()).await;
            this.update(cx, |this, cx| {
                this.config.known_hosts_loading = false;
                match result {
                    Ok(()) => {
                        this.show_toast(ToastKind::Success, format!("Host key removed for {host}"));
                        // Refresh known hosts list
                        let session2 = this.session.clone();
                        cx.spawn(async move |this2, cx2| {
                            let _ = session2.list_known_hosts().await;
                            let _ = this2;
                            let _ = cx2;
                        })
                        .detach();
                    }
                    Err(error) => {
                        this.config.known_hosts_error = Some(error);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn edit_interface_submit(&mut self, cx: &mut Context<Self>) {
        let Some(connected) = self.connected.as_ref() else {
            return;
        };
        if self.config.preview_loading || self.config.apply_loading {
            return;
        }
        let Some(form) = self.interface_edit.as_mut() else {
            self.modal_error = Some("Interface editor is not open.".to_string());
            cx.notify();
            return;
        };
        let name = form.name.read(cx).text().trim().to_string();
        if name.is_empty() {
            self.modal_error = Some("Interface name is required.".to_string());
            cx.notify();
            return;
        }
        let description = form.description.read(cx).text().trim().to_string();
        let mut interface = InterfaceDraft {
            name,
            description: Some(description),
            enabled: Some(form.enabled),
            mode: Some(form.mode.draft_value().to_string()),
            access_vlan: None,
            ipv4_address: None,
            ipv4_prefix_len: None,
            shutdown: None,
        };
        let vlan = form.access_vlan.read(cx).text().trim().to_string();
        if !vlan.is_empty() {
            match vlan.parse::<u16>() {
                Ok(vlan) => interface.access_vlan = Some(vlan),
                Err(_) => {
                    self.modal_error = Some("Access VLAN must be a number.".to_string());
                    cx.notify();
                    return;
                }
            }
        }
        let address = form.ipv4_address.read(cx).text().trim().to_string();
        let prefix = form.ipv4_prefix_len.read(cx).text().trim().to_string();
        if !address.is_empty() || !prefix.is_empty() {
            if address.is_empty() || prefix.is_empty() {
                self.modal_error =
                    Some("IPv4 address and prefix length must be provided together.".to_string());
                cx.notify();
                return;
            }
            if address.parse::<std::net::Ipv4Addr>().is_err() {
                self.modal_error = Some(format!("Invalid IPv4 address: {address}"));
                cx.notify();
                return;
            }
            match prefix.parse::<u8>() {
                Ok(prefix) if prefix <= 32 => {
                    interface.ipv4_address = Some(address);
                    interface.ipv4_prefix_len = Some(prefix);
                }
                _ => {
                    self.modal_error =
                        Some("IPv4 prefix length must be between 0 and 32.".to_string());
                    cx.notify();
                    return;
                }
            }
        }

        let draft = ConfigurationDraft {
            hostname: None,
            domain_name: None,
            add_name_servers: Vec::new(),
            remove_name_servers: Vec::new(),
            add_vlans: Vec::new(),
            remove_vlan_ids: Vec::new(),
            add_interfaces: vec![interface],
            remove_interface_names: Vec::new(),
            add_routes: Vec::new(),
            remove_route_prefixes: Vec::new(),
        };
        let device_id = connected.profile.id.clone();
        let session = self.session.clone();
        self.config.preview_loading = true;
        self.config.preview_error = None;
        self.config.preview = None;
        self.modal_error = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = session.preview_configuration(device_id, draft).await;
            this.update(cx, |this, cx| {
                this.config.preview_loading = false;
                match result {
                    Ok(preview) => {
                        let has_errors = !preview.errors.is_empty();
                        this.config.preview = Some(preview);
                        this.config.preview_error = None;
                        this.close_modal();
                        this.interface_edit = None;
                        this.active = Section::Configuration;
                        this.show_toast(
                            if has_errors {
                                ToastKind::Warning
                            } else {
                                ToastKind::Success
                            },
                            if has_errors {
                                "Preview generated with validation errors"
                            } else {
                                "Interface preview generated"
                            },
                        );
                    }
                    Err(error) => {
                        this.config.preview_error = Some(error.clone());
                        this.modal_error = Some(error);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn edit_vlan_submit(&mut self, cx: &mut Context<Self>) {
        let Some(connected) = self.connected.as_ref() else {
            return;
        };
        if self.config.preview_loading || self.config.apply_loading {
            return;
        }
        let Some(form) = self.vlan_edit.as_mut() else {
            self.modal_error = Some("VLAN editor is not open.".to_string());
            cx.notify();
            return;
        };
        let id_str = form.id.read(cx).text().trim().to_string();
        let id: u16 = match id_str.parse() {
            Ok(id) => id,
            Err(_) => {
                self.modal_error = Some("VLAN ID must be a number.".to_string());
                cx.notify();
                return;
            }
        };
        let name = form.name.read(cx).text().trim().to_string();
        if name.is_empty() {
            self.modal_error = Some("VLAN name is required.".to_string());
            cx.notify();
            return;
        }
        let draft = ConfigurationDraft {
            hostname: None,
            domain_name: None,
            add_name_servers: Vec::new(),
            remove_name_servers: Vec::new(),
            add_vlans: vec![AddVlanDraft {
                id,
                name,
                enabled: Some(form.enabled),
            }],
            remove_vlan_ids: Vec::new(),
            add_interfaces: Vec::new(),
            remove_interface_names: Vec::new(),
            add_routes: Vec::new(),
            remove_route_prefixes: Vec::new(),
        };
        let device_id = connected.profile.id.clone();
        let session = self.session.clone();
        self.config.preview_loading = true;
        self.config.preview_error = None;
        self.config.preview = None;
        self.modal_error = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = session.preview_configuration(device_id, draft).await;
            this.update(cx, |this, cx| {
                this.config.preview_loading = false;
                match result {
                    Ok(preview) => {
                        let has_errors = !preview.errors.is_empty();
                        this.config.preview = Some(preview);
                        this.config.preview_error = None;
                        this.close_modal();
                        this.vlan_edit = None;
                        this.active = Section::Configuration;
                        this.show_toast(
                            if has_errors {
                                ToastKind::Warning
                            } else {
                                ToastKind::Success
                            },
                            if has_errors {
                                "Preview generated with validation errors"
                            } else {
                                "VLAN preview generated"
                            },
                        );
                    }
                    Err(error) => {
                        this.config.preview_error = Some(error.clone());
                        this.modal_error = Some(error);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn edit_route_submit(&mut self, cx: &mut Context<Self>) {
        let Some(connected) = self.connected.as_ref() else {
            return;
        };
        if self.config.preview_loading || self.config.apply_loading {
            return;
        }
        let Some(form) = self.route_edit.as_mut() else {
            self.modal_error = Some("Route editor is not open.".to_string());
            cx.notify();
            return;
        };
        let prefix_str = form.prefix.read(cx).text().trim().to_string();
        let next_hop_str = form.next_hop.read(cx).text().trim().to_string();
        if prefix_str.is_empty() || next_hop_str.is_empty() {
            self.modal_error = Some("Prefix and next hop are required.".to_string());
            cx.notify();
            return;
        }
        if prefix_str.parse::<ipnet::IpNet>().is_err() {
            self.modal_error = Some("Prefix must be CIDR, for example 192.0.2.0/24.".to_string());
            cx.notify();
            return;
        }
        if next_hop_str.parse::<std::net::IpAddr>().is_err() {
            self.modal_error = Some(format!("Invalid next hop: {next_hop_str}"));
            cx.notify();
            return;
        }
        let admin_text = form.admin_distance.read(cx).text().trim().to_string();
        let admin_distance = if admin_text.is_empty() {
            None
        } else {
            match admin_text.parse::<u16>() {
                Ok(distance) if distance >= 1 => Some(distance),
                _ => {
                    self.modal_error =
                        Some("Administrative distance must be a positive number.".to_string());
                    cx.notify();
                    return;
                }
            }
        };
        let remove_route_prefixes = form
            .original_prefix
            .clone()
            .filter(|original| original != &prefix_str)
            .into_iter()
            .collect();
        let draft = ConfigurationDraft {
            hostname: None,
            domain_name: None,
            add_name_servers: Vec::new(),
            remove_name_servers: Vec::new(),
            add_vlans: Vec::new(),
            remove_vlan_ids: Vec::new(),
            add_interfaces: Vec::new(),
            remove_interface_names: Vec::new(),
            add_routes: vec![RouteDraft {
                prefix: prefix_str,
                next_hop: next_hop_str,
                admin_distance,
            }],
            remove_route_prefixes,
        };
        let device_id = connected.profile.id.clone();
        let session = self.session.clone();
        self.config.preview_loading = true;
        self.config.preview_error = None;
        self.config.preview = None;
        self.modal_error = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = session.preview_configuration(device_id, draft).await;
            this.update(cx, |this, cx| {
                this.config.preview_loading = false;
                match result {
                    Ok(preview) => {
                        let has_errors = !preview.errors.is_empty();
                        this.config.preview = Some(preview);
                        this.config.preview_error = None;
                        this.close_modal();
                        this.route_edit = None;
                        this.active = Section::Configuration;
                        this.show_toast(
                            if has_errors {
                                ToastKind::Warning
                            } else {
                                ToastKind::Success
                            },
                            if has_errors {
                                "Preview generated with validation errors"
                            } else {
                                "Route preview generated"
                            },
                        );
                    }
                    Err(error) => {
                        this.config.preview_error = Some(error.clone());
                        this.modal_error = Some(error);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    // ------------------------------------------------------------------
    // Terminal and recording
    // ------------------------------------------------------------------

    fn submit_terminal_command(&mut self, command: String, cx: &mut Context<Self>) {
        let Some(connected) = self.connected.as_ref() else {
            self.terminal.error = Some("connect a device before using the terminal".to_string());
            cx.notify();
            return;
        };
        if self.terminal.busy {
            self.terminal
                .output
                .push_str("[wait for the current command to finish]\n$ ");
            self.terminal.error = None;
            self.terminal.scroll.scroll_to_bottom();
            cx.notify();
            return;
        }
        let command = command.trim().to_string();
        if let Err(error) = validate_device_command(&command) {
            self.terminal.output.push_str(&format!("{error}\n$ "));
            self.terminal.error = Some(error);
            self.terminal.scroll.scroll_to_bottom();
            cx.notify();
            return;
        }

        // Add to command history (avoid duplicates)
        if !self.terminal.command_history.contains(&command) {
            self.terminal.command_history.push(command.clone());
            if self.terminal.command_history.len() > 50 {
                self.terminal.command_history.remove(0);
            }
        }
        self.terminal.history_index = self.terminal.command_history.len();

        self.terminal.output.push_str(&format!("{command}\n"));
        self.terminal.busy = true;
        self.terminal.error = None;
        self.terminal.message = None;
        self.terminal.scroll.scroll_to_bottom();
        cx.notify();

        let session = self.session.clone();
        let device_id = connected.profile.id.clone();
        cx.spawn(async move |this, cx| {
            let result = session.send_command(device_id, command).await;
            this.update(cx, |this, cx| {
                this.terminal.busy = false;
                match result {
                    Ok(output) => {
                        let output = normalize_terminal(&bound_command_output(output));
                        this.terminal.output.push_str(&output);
                        if !output.ends_with('\n') {
                            this.terminal.output.push('\n');
                        }
                        this.terminal.output.push_str("$ ");
                    }
                    Err(error) => {
                        this.terminal.output.push_str(&format!("{error}\n$ "));
                        this.terminal.error = Some(error);
                    }
                }
                this.terminal.scroll.scroll_to_bottom();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn interrupt_terminal(&mut self, cx: &mut Context<Self>) {
        let Some(connected) = self.connected.as_ref() else {
            return;
        };
        let session = self.session.clone();
        let device_id = connected.profile.id.clone();
        self.terminal.error = None;
        self.terminal.message = None;
        cx.spawn(async move |this, cx| {
            let result = session.send_interrupt(device_id).await;
            this.update(cx, |this, cx| {
                this.terminal.output.push_str("^C\n$ ");
                match result {
                    Ok(()) => {
                        this.terminal.message = Some("Interrupt sent to the device.".to_string())
                    }
                    Err(error) => this.terminal.error = Some(error),
                }
                this.terminal.scroll.scroll_to_bottom();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn toggle_recording(&mut self, cx: &mut Context<Self>) {
        let Some(connected) = self.connected.as_ref() else {
            return;
        };
        if self.terminal.recording_busy {
            return;
        }
        let session = self.session.clone();
        let device_id = connected.profile.id.clone();
        let active = self
            .terminal
            .recording
            .as_ref()
            .is_some_and(|status| status.active);
        self.terminal.recording_busy = true;
        self.terminal.error = None;
        self.terminal.message = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = if active {
                session
                    .stop_recording(device_id)
                    .await
                    .map(|status| (status, "Recording stopped."))
            } else {
                session
                    .start_recording(device_id)
                    .await
                    .map(|status| (Some(status), "Recording started."))
            };
            this.update(cx, |this, cx| {
                this.terminal.recording_busy = false;
                match result {
                    Ok((status, message)) => {
                        this.terminal.recording = status;
                        this.terminal.message = Some(message.to_string());
                    }
                    Err(error) => this.terminal.error = Some(error),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn refresh_recording(&mut self, cx: &mut Context<Self>) {
        let Some(connected) = self.connected.as_ref() else {
            return;
        };
        let session = self.session.clone();
        let device_id = connected.profile.id.clone();
        cx.spawn(async move |this, cx| {
            let result = session.recording_status(device_id).await;
            this.update(cx, |this, cx| {
                if let Ok(status) = result {
                    this.terminal.recording = status;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn clear_terminal(&mut self, cx: &mut Context<Self>) {
        self.terminal.output.clear();
        self.terminal.output.push_str("$ ");
        self.terminal.message =
            Some("Terminal view cleared. Log files were not deleted.".to_string());
        self.terminal.error = None;
        cx.notify();
    }

    fn copy_terminal(&mut self, cx: &mut Context<Self>) {
        let text = self.terminal.output.trim_end().to_string();
        if text.is_empty() {
            self.terminal.message = Some("Nothing to copy yet.".to_string());
        } else {
            let length = text.chars().count();
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            self.terminal.message = Some(format!("Copied {length} characters to the clipboard."));
        }
        self.terminal.error = None;
        cx.notify();
    }

    // ------------------------------------------------------------------
    // Diagnostics
    // ------------------------------------------------------------------

    fn refresh_diagnostics(&mut self, cx: &mut Context<Self>) {
        if self.diagnostics_ui.loading {
            return;
        }
        let diagnostics = Arc::clone(&self.diagnostics);
        self.diagnostics_ui.loading = true;
        self.diagnostics_ui.error = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { diagnostics.status() })
                .await;
            this.update(cx, |this, cx| {
                this.diagnostics_ui.loading = false;
                match result {
                    Ok(status) => {
                        this.diagnostics_ui.status = Some(status);
                        this.diagnostics_ui.error = None;
                    }
                    Err(error) => this.diagnostics_ui.error = Some(error),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn refresh_log_files(&mut self, cx: &mut Context<Self>) {
        if self.diagnostics_ui.files_loading {
            return;
        }
        let diagnostics = Arc::clone(&self.diagnostics);
        self.diagnostics_ui.files_loading = true;
        self.diagnostics_ui.files_error = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { diagnostics.list_files() })
                .await;
            this.update(cx, |this, cx| {
                this.diagnostics_ui.files_loading = false;
                match result {
                    Ok(files) => {
                        this.diagnostics_ui.files = files;
                        this.diagnostics_ui.files_error = None;
                    }
                    Err(error) => this.diagnostics_ui.files_error = Some(error),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn view_log(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(file) = self.diagnostics_ui.files.get(index) else {
            return;
        };
        let path = file.path.clone();
        let diagnostics = Arc::clone(&self.diagnostics);
        self.diagnostics_ui.view_error = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { diagnostics.read_file(&path) })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(content) => {
                        this.diagnostics_ui.selected = Some(content);
                        this.diagnostics_ui.view_error = None;
                    }
                    Err(error) => this.diagnostics_ui.view_error = Some(error),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn copy_log(&mut self, cx: &mut Context<Self>) {
        let Some(log) = self.diagnostics_ui.selected.as_ref() else {
            return;
        };
        let content = log.content.clone();
        let path = log.path.clone();
        let length = content.chars().count();
        cx.write_to_clipboard(ClipboardItem::new_string(content));
        self.diagnostics_ui.message = Some(format!("Copied {length} characters from {path}."));
        self.diagnostics_ui.view_error = None;
        cx.notify();
    }

    fn export_diagnostics(&mut self, cx: &mut Context<Self>) {
        if self.diagnostics_ui.busy {
            return;
        }
        let diagnostics = Arc::clone(&self.diagnostics);
        self.diagnostics_ui.busy = true;
        self.diagnostics_ui.error = None;
        self.diagnostics_ui.message = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    diagnostics
                        .export()
                        .map(|path| path.to_string_lossy().to_string())
                })
                .await;
            this.update(cx, |this, cx| {
                this.diagnostics_ui.busy = false;
                match result {
                    Ok(path) => {
                        this.diagnostics_ui.message =
                            Some(format!("Diagnostic ZIP created: {path}"));
                    }
                    Err(error) => this.diagnostics_ui.error = Some(error),
                }
                this.refresh_log_files(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn open_logs_directory(&mut self, cx: &mut Context<Self>) {
        if self.diagnostics_ui.busy {
            return;
        }
        let diagnostics = Arc::clone(&self.diagnostics);
        self.diagnostics_ui.busy = true;
        self.diagnostics_ui.error = None;
        self.diagnostics_ui.message = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    diagnostics
                        .open_logs_directory()
                        .map(|path| path.to_string_lossy().to_string())
                })
                .await;
            this.update(cx, |this, cx| {
                this.diagnostics_ui.busy = false;
                match result {
                    Ok(path) => {
                        this.diagnostics_ui.message = Some(format!("Opened log directory: {path}"))
                    }
                    Err(error) => this.diagnostics_ui.error = Some(error),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn save_test_note(&mut self, cx: &mut Context<Self>) {
        if self.diagnostics_ui.busy {
            return;
        }
        let note = self
            .diagnostics_ui
            .note
            .update(cx, |field, cx| field.take_content(cx));
        if note.trim().is_empty() {
            self.diagnostics_ui.error = Some("test note cannot be empty".to_string());
            cx.notify();
            return;
        }
        let diagnostics = Arc::clone(&self.diagnostics);
        self.diagnostics_ui.busy = true;
        self.diagnostics_ui.error = None;
        self.diagnostics_ui.message = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    diagnostics
                        .test_note(&note)
                        .map(|path| path.to_string_lossy().to_string())
                })
                .await;
            this.update(cx, |this, cx| {
                this.diagnostics_ui.busy = false;
                match result {
                    Ok(path) => {
                        this.diagnostics_ui.message = Some(format!("Test note saved to {path}"));
                        this.refresh_log_files(cx);
                    }
                    Err(error) => this.diagnostics_ui.error = Some(error),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    // ------------------------------------------------------------------
    // Rendering helpers
    // ------------------------------------------------------------------

    fn nav_item(active: bool, section: Section, cx: &mut Context<Self>) -> AnyElement {
        let mut item = div()
            .id(("nav", section.index()))
            .flex()
            .items_center()
            .gap_2()
            .w_full()
            .px_3()
            .py_2()
            .rounded_md()
            .cursor_pointer()
            .text_xs()
            .child(
                div()
                    .w(px(18.))
                    .flex_shrink_0()
                    .text_color(if active {
                        palette::accent()
                    } else {
                        palette::text_faint()
                    })
                    .child(section.icon().to_string()),
            )
            .child(section.label().to_string());

        if active {
            item = item
                .bg(palette::selection())
                .text_color(palette::accent_strong())
                .font_weight(FontWeight::SEMIBOLD);
        } else {
            item = item
                .text_color(palette::text_muted())
                .hover(|style| style.bg(palette::panel_alt()).text_color(palette::text()));
        }

        item.on_click(cx.listener(move |this, _, _, cx| this.navigate(section, cx)))
            .into_any_element()
    }

    fn group_header(group: NavGroup, collapsed: bool, cx: &mut Context<Self>) -> AnyElement {
        let arrow = if collapsed { "▸" } else { "▾" };
        div()
            .flex()
            .items_center()
            .gap_2()
            .w_full()
            .px_3()
            .py_2()
            .cursor_pointer()
            .text_xs()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(palette::text_faint())
            .child(arrow)
            .child(group.label())
            .child(
                div()
                    .flex_1()
                    .child(div().h(px(1.)).bg(palette::panel_border())),
            );
        let mut header = div()
            .flex()
            .items_center()
            .gap_2()
            .w_full()
            .px_3()
            .py_2()
            .cursor_pointer()
            .text_xs()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(palette::text_faint())
            .child(arrow)
            .child(group.label())
            .child(
                div()
                    .flex_1()
                    .child(div().h(px(1.)).bg(palette::panel_border())),
            );
        let weak = cx.entity().downgrade();
        header = header.on_mouse_down(MouseButton::Left, move |event, window, app| {
            let _ = event;
            let _ = window;
            weak.update(app, |this, cx| {
                let index = group.index();
                this.collapsed_groups[index] = !this.collapsed_groups[index];
                cx.notify();
            })
            .ok();
        });
        header.into_any_element()
    }

    fn group_items(group: NavGroup, cx: &mut Context<Self>) -> Vec<Section> {
        Section::ALL
            .iter()
            .copied()
            .filter(|section| section.group() == group)
            .collect()
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let width = if self.sidebar_collapsed {
            px(72.)
        } else {
            px(224.)
        };
        let connected = self.connected.is_some();

        let mut groups_content = div().flex().flex_col().gap_1().w_full();
        for group in NavGroup::ALL {
            let collapsed = self.collapsed_groups[group.index()];
            groups_content = groups_content.child(Self::group_header(group, collapsed, cx));
            if !collapsed {
                for section in Self::group_items(group, cx) {
                    groups_content =
                        groups_content.child(Self::nav_item(self.active == section, section, cx));
                }
            }
        }

        let status_label = self
            .connected
            .as_ref()
            .map(|connected| connected.profile.name.clone())
            .unwrap_or_else(|| "Not connected".to_string());
        let status_color = if connected {
            palette::accent()
        } else {
            palette::text_faint()
        };

        let mut sidebar = div()
            .w(width)
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_4()
            .p_3()
            .border_r_1()
            .border_color(palette::panel_border())
            .bg(palette::sidebar());

        let mut logo = div()
            .flex()
            .items_center()
            .gap_2()
            .pb_3()
            .cursor_pointer()
            .child(
                div()
                    .size(px(24.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(palette::accent())
                    .text_color(palette::background())
                    .font_weight(FontWeight::BOLD)
                    .child("r"),
            )
            .child(
                div()
                    .text_xl()
                    .font_weight(FontWeight::BOLD)
                    .text_color(palette::text())
                    .truncate()
                    .child("reef."),
            );
        let weak = cx.entity().downgrade();
        logo = logo.on_mouse_down(MouseButton::Left, move |event, window, app| {
            let _ = event;
            let _ = window;
            weak.update(app, |this, cx| {
                this.sidebar_collapsed = !this.sidebar_collapsed;
                cx.notify();
            })
            .ok();
        });
        sidebar = sidebar.child(logo);

        if !self.sidebar_collapsed {
            sidebar = sidebar.child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(palette::text_faint())
                    .child("WORKSPACE"),
            );
            sidebar = sidebar.child(groups_content);
            sidebar = sidebar.child(div().flex_1());
        }

        sidebar = sidebar.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .py_2()
                .rounded_md()
                .bg(palette::panel())
                .child(div().size(px(8.)).rounded_full().bg(status_color))
                .child(
                    div()
                        .text_xs()
                        .text_color(if connected {
                            palette::text()
                        } else {
                            palette::text_muted()
                        })
                        .truncate()
                        .child(status_label),
                ),
        );

        sidebar.into_any_element()
    }

    fn topbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let right = if let Some(connected) = self.connected.as_ref() {
            let name = connected.profile.name.clone();
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_sm()
                        .text_color(palette::text_muted())
                        .child(div().size(px(8.)).rounded_full().bg(palette::accent()))
                        .child(name),
                )
                .child(Self::action_button(
                    "Disconnect",
                    true,
                    false,
                    cx,
                    ReefApp::disconnect,
                ))
                .into_any_element()
        } else {
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_sm()
                .text_color(palette::text_muted())
                .child(div().size(px(8.)).rounded_full().bg(palette::text_faint()))
                .child("No device connected")
                .into_any_element()
        };

        div()
            .h(px(56.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_between()
            .px_6()
            .border_b_1()
            .border_color(palette::panel_border())
            .bg(palette::sidebar())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_sm()
                    .text_color(palette::text_muted())
                    .child("Workspace")
                    .child(
                        div()
                            .text_color(palette::text_faint())
                            .child("/")
                            .child(self.active.label().to_string()),
                    ),
            )
            .child(right)
            .into_any_element()
    }

    fn page_heading(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let is_connected = self.connected.is_some();
        let connect_enabled = is_connected
            && matches!(
                self.active,
                Section::Overview | Section::Interfaces | Section::Vlans | Section::Routing
            );

        div()
            .flex()
            .items_end()
            .justify_between()
            .gap_4()
            .w_full()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(palette::accent())
                            .child("NETWORK WORKSPACE"),
                    )
                    .child(
                        div()
                            .text_2xl()
                            .font_weight(FontWeight::BOLD)
                            .text_color(palette::text())
                            .child(self.active.label().to_string()),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(palette::text_muted())
                            .child(self.active.subtitle()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .gap_2()
                    .child(Self::action_button(
                        "＋ Connect device",
                        true,
                        true,
                        cx,
                        |this, cx| this.navigate(Section::Connections, cx),
                    ))
                    .child(Self::action_button(
                        "＋ Edit",
                        connect_enabled,
                        false,
                        cx,
                        move |this, cx| match this.active {
                            Section::Interfaces => {
                                this.open_interface_modal(None, cx);
                            }
                            Section::Vlans => {
                                this.open_vlan_modal(None, cx);
                            }
                            Section::Routing => {
                                this.open_route_modal(None, cx);
                            }
                            _ => {}
                        },
                    ))
                    .into_any_element(),
            )
            .into_any_element()
    }

    fn action_button<F>(
        label: impl Into<SharedString>,
        enabled: bool,
        primary: bool,
        cx: &mut Context<Self>,
        action: F,
    ) -> AnyElement
    where
        F: Fn(&mut Self, &mut Context<Self>) + 'static,
    {
        let label: SharedString = label.into();
        let mut button = div()
            .id(ElementId::Name(label.clone()))
            .flex()
            .items_center()
            .justify_center()
            .h(px(34.))
            .px_3()
            .rounded_md()
            .text_sm()
            .font_weight(FontWeight::SEMIBOLD)
            .child(label);

        if primary {
            button = button
                .bg(palette::accent())
                .text_color(palette::background());
        } else {
            button = button
                .bg(palette::panel_alt())
                .text_color(palette::text())
                .border_1()
                .border_color(palette::panel_border());
        }

        if enabled {
            button = button
                .cursor_pointer()
                .hover(|style| style.opacity(0.85))
                .on_click(cx.listener(move |this, _, _, cx| action(this, cx)));
        } else {
            button = button.opacity(0.5);
        }

        button.into_any_element()
    }

    fn field_block(label: impl Into<SharedString>, field: Entity<TextField>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_1()
            .w_full()
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(palette::text_muted())
                    .child(label.into()),
            )
            .child(field)
            .into_any_element()
    }

    fn section_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        match self.active {
            Section::Overview => self.overview_body(cx),
            Section::Connections => self.connections_body(cx),
            Section::Terminal => self.terminal_body(cx),
            Section::Diagnostics => self.diagnostics_body(cx),
            Section::Interfaces | Section::Vlans | Section::Routing => self.workspace_body(cx),
            Section::Configuration => self.configuration_body(cx),
            Section::AccessLists | Section::Dhcp | Section::System => {
                self.configuration_topic_body(cx)
            }
            Section::Nat | Section::SshAccess => self.configuration_security_body(cx),
        }
    }

    // ------------------------------------------------------------------
    // Overview
    // ------------------------------------------------------------------

    fn overview_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(connected) = self.connected.as_ref() else {
            return panel(
                "Your network workspace",
                "Connect to a device to inspect its interfaces, manage configuration, and monitor its status.",
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .items_start()
                    .child(paragraph(
                        "Profiles store only the device address and transport. Passwords are requested for each connection and are never saved.",
                    ))
                    .child(Self::action_button(
                        "Connect your first device →",
                        true,
                        true,
                        cx,
                        |this, cx| this.navigate(Section::Connections, cx),
                    ))
                    .into_any_element(),
            );
        };

        let name = connected.profile.name.clone();
        let host = connected.profile.host.clone();
        let port = connected.profile.port;
        let transport = transport_label(&connected.profile.transport);
        let username = connected.profile.username.clone();
        let identity = connected.identity.clone();
        let identity_error = connected.identity_error.clone();
        let enabling = connected.enabling;
        let privilege_secret = connected.privilege_secret.clone();
        let connected_name = connected.profile.name.clone();

        let mut body = div().flex().flex_col().gap_3().w_full();
        if let Some(identity) = identity {
            let up = identity
                .interfaces
                .iter()
                .filter(|interface| {
                    interface
                        .oper_status
                        .as_deref()
                        .is_some_and(|status| status.eq_ignore_ascii_case("up"))
                })
                .count();
            let down = identity
                .interfaces
                .iter()
                .filter(|interface| {
                    interface
                        .oper_status
                        .as_deref()
                        .is_some_and(|status| status.eq_ignore_ascii_case("down"))
                })
                .count();

            body = body
                .child(
                    div()
                        .text_sm()
                        .text_color(palette::accent_strong())
                        .child(format!(
                            "{} · {} · IOS XE {}",
                            identity.hostname, identity.platform, identity.iosxe_version
                        )),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .child(stat_pill(&format!(
                            "{} discovered interfaces",
                            identity.interfaces.len()
                        )))
                        .child(stat_pill(&format!("{up} operationally up")))
                        .child(stat_pill(&format!("{down} down")))
                        .child(stat_pill(&format!(
                            "{} processor",
                            if identity.processor.is_empty() {
                                "Unknown"
                            } else {
                                &identity.processor
                            }
                        )))
                        .child(stat_pill(&format!(
                            "{} chassis serial",
                            if identity.chassis_serial.is_empty() {
                                "Unavailable"
                            } else {
                                &identity.chassis_serial
                            }
                        ))),
                )
                .child(capability_grid(&identity))
                .child(
                    div()
                        .text_xs()
                        .text_color(palette::text_faint())
                        .child("DEVICE IDENTIFIED · LIVE INSPECTION AVAILABLE"),
                );
        } else if let Some(error) = identity_error {
            body = body
                .child(error_panel(&format!(
                    "Connected, but device identification failed: {error}"
                )))
                .child(Self::field_block("Enable secret", privilege_secret))
                .child(Self::action_button(
                    if enabling {
                        "Enabling…"
                    } else {
                        "Enable and identify"
                    },
                    !enabling,
                    true,
                    cx,
                    ReefApp::enable_and_identify,
                ));
        } else {
            body = body.child(paragraph(
                "Session established. Identifying the device through its CLI…",
            ));
        }

        let primary = panel(
            &format!("Connected to {name}"),
            &format!("{host}:{port} · {transport} · {username}"),
            body.into_any_element(),
        );

        let quick = div()
            .flex()
            .flex_wrap()
            .gap_3()
            .w_full()
            .child(quick_card("Interfaces", Section::Interfaces, cx))
            .child(quick_card("VLANs", Section::Vlans, cx))
            .child(quick_card("Routing", Section::Routing, cx))
            .child(quick_card("Configuration", Section::Configuration, cx))
            .into_any_element();

        let _ = connected_name;
        div()
            .flex()
            .flex_col()
            .gap_4()
            .w_full()
            .child(primary)
            .child(quick)
            .into_any_element()
    }

    // ------------------------------------------------------------------
    // Connections
    // ------------------------------------------------------------------

    fn connections_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mut list = div().flex().flex_col().gap_2().w_full();

        if self.profiles_loading && self.profiles.is_empty() {
            list = list.child(paragraph("Loading saved profiles…"));
        } else if let Some(error) = self.profile_error.clone() {
            list = list.child(error_panel(&format!("Unable to load profiles: {error}")));
        } else if self.profiles.is_empty() {
            list = list.child(paragraph(
                "No saved profiles yet. Create a profile to save a device's address and connection preferences.",
            ));
        } else {
            let profiles = self.profiles.clone();
            for (index, profile) in profiles.into_iter().enumerate() {
                let is_selected = self
                    .connection
                    .as_ref()
                    .is_some_and(|form| form.profile.id == profile.id);
                let row = div()
                    .id(("profile-row", index))
                    .flex()
                    .items_center()
                    .gap_3()
                    .w_full()
                    .p_3()
                    .rounded_md()
                    .border_1()
                    .border_color(if is_selected {
                        palette::accent_dim()
                    } else {
                        palette::panel_border()
                    })
                    .bg(if is_selected {
                        palette::selection()
                    } else {
                        palette::panel_alt()
                    })
                    .child(div().text_color(palette::accent()).child("◇").w(px(20.)))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .gap_0()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(profile.name.clone()),
                            )
                            .child(div().text_xs().text_color(palette::text_muted()).child(
                                format!(
                                    "{}:{} · {} · {}",
                                    profile.host,
                                    profile.port,
                                    transport_label(&profile.transport),
                                    if profile.username.is_empty() {
                                        "No username"
                                    } else {
                                        &profile.username
                                    }
                                ),
                            )),
                    )
                    .child(Self::action_button("Connect", true, false, cx, {
                        let profile = profile.clone();
                        move |this, cx| this.select_profile(profile.clone(), cx)
                    }))
                    .child(Self::action_button("×", true, false, cx, {
                        let profile = profile.clone();
                        move |this, cx| this.delete_profile(profile.clone(), cx)
                    }));
                list = list.child(row);
            }
        }

        let profiles_panel = panel(
            "Saved profiles",
            "Connection details are stored locally. Passwords are never saved in profiles.",
            div()
                .flex()
                .flex_col()
                .gap_3()
                .w_full()
                .child(div().flex().justify_end().child(Self::action_button(
                    if self.profiles_loading {
                        "Refreshing…"
                    } else {
                        "Refresh"
                    },
                    !self.profiles_loading,
                    false,
                    cx,
                    ReefApp::refresh_profiles,
                )))
                .child(list)
                .into_any_element(),
        );

        let connection_form = self.connection.as_ref().map(|form| {
            let profile = form.profile.clone();
            let is_serial = profile.transport == ProfileTransport::Serial;
            let error = form.error.clone();
            let connecting = form.connecting;
            let trust = form.trust_host_key;
            let password = form.password.clone();
            let enable_secret = form.enable_secret.clone();

            let mut auth = div().flex().flex_col().gap_3().w_full();
            if !is_serial {
                auth = auth
                    .child(Self::field_block("Password", password))
                    .child(Self::field_block("Enable secret (optional)", enable_secret));
            }
            if let Some(error) = error {
                let unknown_key = error.to_lowercase().contains("unknown host key");
                auth = auth.child(error_panel(&error));
                if unknown_key && profile.transport == ProfileTransport::Ssh {
                    auth = auth.child(checkbox_row(
                        "Trust this SSH host key and save it for future connections",
                        trust,
                        cx,
                    ));
                }
            }
            auth = auth
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .justify_end()
                        .child(Self::action_button("Cancel", true, false, cx, |this, cx| {
                            this.connection = None;
                            cx.notify();
                        }))
                        .child(Self::action_button(
                            if connecting { "Connecting…" } else { "Connect" },
                            !connecting,
                            true,
                            cx,
                            ReefApp::connect_selected,
                        )),
                )
                .child(paragraph(
                    "Credentials are sent to the Rust session service for this connection only. They are not written to the profile.",
                ));

            panel(
                &format!("Connect to {}", profile.name),
                &format!(
                    "{} · {}",
                    profile.host,
                    transport_label(&profile.transport)
                ),
                auth.into_any_element(),
            )
        });

        let form = &self.profile_form;
        let mut transport_buttons = div().flex().gap_2().w_full();
        for choice in [
            TransportChoice::Ssh,
            TransportChoice::Telnet,
            TransportChoice::Serial,
        ] {
            let active = form.transport == choice;
            let mut button = div()
                .flex()
                .items_center()
                .justify_center()
                .h(px(32.))
                .px_3()
                .rounded_md()
                .text_sm()
                .cursor_pointer()
                .child(choice.label().to_string());
            if active {
                button = button
                    .bg(palette::accent_dim())
                    .text_color(palette::accent_strong())
                    .font_weight(FontWeight::SEMIBOLD);
            } else {
                button = button
                    .bg(palette::panel_alt())
                    .text_color(palette::text_muted())
                    .border_1()
                    .border_color(palette::panel_border());
            }
            transport_buttons = transport_buttons.child(
                button
                    .id(ElementId::Name(choice.label().into()))
                    .on_click(cx.listener(move |this, _, _, cx| this.select_transport(choice, cx))),
            );
        }

        let mut new_profile = div()
            .flex()
            .flex_col()
            .gap_3()
            .w_full()
            .child(Self::field_block("Profile name", form.name.clone()))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .w_full()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(palette::text_muted())
                            .child("Transport"),
                    )
                    .child(transport_buttons),
            )
            .child(Self::field_block(
                if form.transport.is_serial() {
                    "Serial port"
                } else {
                    "Host or IP address"
                },
                form.host.clone(),
            ));

        if !form.transport.is_serial() {
            new_profile = new_profile
                .child(Self::field_block("Port", form.port.clone()))
                .child(Self::field_block("Username", form.username.clone()));
        }
        if let Some(error) = form.error.clone() {
            new_profile = new_profile.child(error_panel(&error));
        }
        new_profile = new_profile.child(div().flex().justify_end().child(Self::action_button(
            if form.saving {
                "Saving…"
            } else {
                "Save profile"
            },
            !form.saving,
            true,
            cx,
            ReefApp::save_profile,
        )));

        let new_panel = panel(
            "New connection profile",
            "Save a reusable device profile. Authentication secrets are requested separately when connecting.",
            new_profile.into_any_element(),
        );

        let mut layout = div().flex().flex_col().gap_4().w_full();
        layout = layout.child(profiles_panel);
        if let Some(connection_form) = connection_form {
            layout = layout.child(connection_form);
        }
        layout.child(new_panel).into_any_element()
    }

    // ------------------------------------------------------------------
    // Operational workspace (interfaces, VLANs, routes)
    // ------------------------------------------------------------------

    fn workspace_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(connected) = self.connected.as_ref() else {
            return empty_connected_panel(
                "Connect a device first",
                "Device data is available after establishing a privileged session.",
                cx,
            );
        };
        let name = connected.profile.name.clone();
        let mut layout = div().flex().flex_col().gap_4().w_full();
        layout = layout.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_4()
                .w_full()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(heading(name))
                        .child(paragraph(
                            "Live operational data queried from the connected device.",
                        )),
                )
                .child(Self::action_button(
                    if self.workspace.loading {
                        "Refreshing…"
                    } else {
                        "Refresh data"
                    },
                    !self.workspace.loading,
                    false,
                    cx,
                    ReefApp::refresh_workspace,
                )),
        );

        if let Some(error) = self.workspace.error.clone() {
            return layout
                .child(error_panel(&format!("Unable to inspect device: {error}")))
                .into_any_element();
        }
        if self.workspace.loading && self.workspace.data.is_none() {
            return layout
                .child(paragraph("Querying device operational state…"))
                .into_any_element();
        }

        let Some(workspace) = self.workspace.data.as_ref() else {
            return layout
                .child(paragraph("No operational data has been loaded yet."))
                .into_any_element();
        };

        let body = match self.active {
            Section::Interfaces => {
                let mut content = div().flex().flex_col().gap_2().w_full();
                if let Some(error) = workspace.interface_error.as_ref() {
                    content = content.child(error_panel(error));
                } else if workspace.interfaces.is_empty() {
                    content = content.child(paragraph(
                        "No interface rows were recognized in the device output.",
                    ));
                } else {
                    content = content.child(table_header(&["Interface", "Operational status"]));
                    for interface in &workspace.interfaces {
                        let status = interface
                            .oper_status
                            .clone()
                            .unwrap_or_else(|| "Unknown".into());
                        content = content.child(table_row(
                            vec![interface.name.clone(), status.clone()],
                            status.eq_ignore_ascii_case("up"),
                        ));
                    }
                }
                panel(
                    "Interfaces",
                    &format!(
                        "{} interfaces returned by IOS CLI",
                        workspace.interfaces.len()
                    ),
                    content.into_any_element(),
                )
            }
            Section::Vlans => {
                let mut content = div().flex().flex_col().gap_2().w_full();
                if let Some(error) = workspace.vlan_error.as_ref() {
                    content = content.child(error_panel(error));
                } else if workspace.vlans.is_empty() {
                    content = content.child(paragraph(
                        "No VLANs were recognized. This command may not be supported on this device.",
                    ));
                } else {
                    content = content.child(table_header(&["VLAN ID", "Name", "Status"]));
                    for vlan in &workspace.vlans {
                        content = content.child(table_row(
                            vec![
                                vlan.id.to_string(),
                                vlan.name.clone(),
                                if vlan.enabled {
                                    "Active".to_string()
                                } else {
                                    "Suspended".to_string()
                                },
                            ],
                            vlan.enabled,
                        ));
                    }
                }
                panel(
                    "VLANs",
                    &format!("{} VLANs returned by IOS CLI", workspace.vlans.len()),
                    content.into_any_element(),
                )
            }
            _ => {
                let mut content = div().flex().flex_col().gap_2().w_full();
                if let Some(error) = workspace.route_error.as_ref() {
                    content = content.child(error_panel(error));
                } else if workspace.static_routes.is_empty() {
                    content = content.child(paragraph(
                        "No static routes were recognized. Connected and dynamic routes are not shown in this initial view.",
                    ));
                } else {
                    content = content.child(table_header(&[
                        "Destination",
                        "Next hop",
                        "Administrative distance",
                    ]));
                    for route in &workspace.static_routes {
                        content = content.child(table_row(
                            vec![
                                route.prefix.to_string(),
                                route.next_hop.to_string(),
                                route
                                    .admin_distance
                                    .map(|distance| distance.to_string())
                                    .unwrap_or_else(|| "Default".to_string()),
                            ],
                            false,
                        ));
                    }
                }
                panel(
                    "Static routes",
                    &format!(
                        "{} static routes parsed from the routing table",
                        workspace.static_routes.len()
                    ),
                    content.into_any_element(),
                )
            }
        };

        layout.child(body).into_any_element()
    }

    // ------------------------------------------------------------------
    // Configuration
    // ------------------------------------------------------------------

    fn configuration_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(connected) = self.connected.as_ref() else {
            return empty_connected_panel(
                "Connect a device first",
                "Running configuration is retrieved from an active device session.",
                cx,
            );
        };
        let name = connected.profile.name.clone();
        let mut layout = div()
            .flex()
            .flex_col()
            .gap_4()
            .w_full()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .w_full()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(heading("Running configuration"))
                            .child(paragraph(format!(
                                "{name} · read-only structured snapshot. Secrets are excluded from this view."
                            ))),
                    )
                    .child(Self::action_button(
                        if self.config.loading {
                            "Refreshing…"
                        } else {
                            "Refresh configuration"
                        },
                        !self.config.loading,
                        false,
                        cx,
                        ReefApp::refresh_configuration,
                    )),
            );

        if let Some(error) = self.config.error.clone() {
            return layout
                .child(error_panel(&format!(
                    "Unable to retrieve configuration: {error}"
                )))
                .into_any_element();
        }
        if self.config.loading && self.config.data.is_none() {
            return layout
                .child(paragraph("Retrieving and parsing running configuration…"))
                .into_any_element();
        }
        let Some(config) = self.config.data.as_ref() else {
            return layout
                .child(paragraph("No configuration snapshot has been loaded yet."))
                .into_any_element();
        };

        let hostname_placeholder = config.hostname.clone().unwrap_or_default();
        let domain_placeholder = config.domain_name.clone().unwrap_or_default();
        let mut draft = div()
            .flex()
            .flex_col()
            .gap_3()
            .w_full()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_3()
                    .w_full()
                    .child(Self::field_block(
                        format!("New hostname ({hostname_placeholder})"),
                        self.draft.hostname.clone(),
                    ))
                    .child(Self::field_block(
                        format!("Domain name ({domain_placeholder})"),
                        self.draft.domain_name.clone(),
                    ))
                    .child(Self::field_block(
                        "Name servers",
                        self.draft.name_servers.clone(),
                    )),
            )
            .child(paragraph(
                "Use the + Edit buttons in the Interfaces, VLANs, and Routing pages to stage changes, then generate a preview from this page.",
            ));

        let has_draft = !self.draft.hostname.read(cx).is_empty()
            || !self.draft.domain_name.read(cx).is_empty()
            || !self.draft.name_servers.read(cx).is_empty();

        draft = draft.child(div().flex().justify_end().child(Self::action_button(
            if self.config.preview_loading {
                "Generating preview…"
            } else {
                "Generate configuration preview"
            },
            has_draft && !self.config.preview_loading,
            true,
            cx,
            ReefApp::preview_configuration,
        )));

        if let Some(error) = self.config.preview_error.clone() {
            draft = draft.child(error_panel(&error));
        }
        if let Some(preview) = self.config.preview.as_ref() {
            if !preview.errors.is_empty() {
                let mut issues = div().flex().flex_col().gap_1().w_full();
                for issue in &preview.errors {
                    issues = issues.child(paragraph(issue.clone()));
                }
                draft = draft.child(error_panel_with_body(
                    "Preview has validation errors",
                    issues.into_any_element(),
                ));
            }
            for warning in &preview.warnings {
                draft = draft.child(notice_panel(warning.clone()));
            }
            draft = draft
                .child(section_label("Proposed diff"))
                .child(code_block("config-diff", preview.diff.clone()))
                .child(section_label("Generated IOS XE commands"))
                .child(code_block(
                    "config-commands",
                    if preview.commands.is_empty() {
                        "No commands generated.".to_string()
                    } else {
                        preview.commands.join("\n")
                    },
                ));
            if preview.affects_connectivity {
                draft = draft.child(paragraph(
                    "This preview may affect management connectivity.",
                ));
            }
            draft = draft.child(paragraph(
                "Preview only. No commands are sent to the device and startup configuration is not modified.",
            ));
            // Apply configuration button
            if let Some(message) = &self.config.apply_message {
                draft = draft.child(notice_panel(message.clone()));
            }
            if let Some(error) = &self.config.apply_error {
                draft = draft.child(error_panel(&error.clone()));
            }
            draft = draft.child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(Self::action_button(
                        if self.config.apply_loading {
                            "Applying…"
                        } else {
                            "Apply configuration to device"
                        },
                        !self.config.apply_loading
                            && !preview.commands.is_empty()
                            && !preview.affects_connectivity,
                        true,
                        cx,
                        ReefApp::apply_configuration,
                    )),
            );
        }

        layout = layout
            .child(panel(
                "System settings preview",
                "Set-only preview. Blank system fields preserve current values; name servers are additive.",
                draft.into_any_element(),
            ))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_3()
                    .w_full()
                    .child(stat_card(
                        "HOSTNAME",
                        config.hostname.clone().unwrap_or_else(|| "Not configured".into()),
                    ))
                    .child(stat_card(
                        "DOMAIN NAME",
                        config
                            .domain_name
                            .clone()
                            .unwrap_or_else(|| "Not configured".into()),
                    ))
                    .child(stat_card("INTERFACES", config.interfaces.len().to_string()))
                    .child(stat_card("VLANS", config.vlans.len().to_string())),
            );

        let mut interfaces = div().flex().flex_col().gap_2().w_full();
        if config.interfaces.is_empty() {
            interfaces = interfaces.child(paragraph("No interface configuration was parsed."));
        } else {
            interfaces = interfaces.child(table_header(&[
                "Interface",
                "Description",
                "Mode",
                "IPv4",
                "Admin state",
            ]));
            for interface in &config.interfaces {
                interfaces = interfaces.child(table_row(
                    vec![
                        interface.name.clone(),
                        interface.description.clone().unwrap_or_else(|| "—".into()),
                        interface.mode.clone(),
                        interface.ipv4.clone().unwrap_or_else(|| "—".into()),
                        if interface.enabled {
                            "enabled".to_string()
                        } else {
                            "shutdown".to_string()
                        },
                    ],
                    interface.enabled,
                ));
            }
        }
        layout = layout.child(panel(
            "Configured interfaces",
            "Configuration state, not live link state",
            interfaces.into_any_element(),
        ));

        layout = layout.child(
            div()
                .flex()
                .flex_wrap()
                .gap_4()
                .w_full()
                .child(Self::action_button(
                    "+ Add interface",
                    true,
                    false,
                    cx,
                    |this, cx| {
                        this.open_interface_modal(None, cx);
                    },
                ))
                .child(Self::action_button(
                    "+ Add VLAN",
                    true,
                    false,
                    cx,
                    |this, cx| {
                        this.open_vlan_modal(None, cx);
                    },
                ))
                .child(Self::action_button(
                    "+ Add route",
                    true,
                    false,
                    cx,
                    |this, cx| {
                        this.open_route_modal(None, cx);
                    },
                ))
                .into_any_element(),
        );

        let mut other = div().flex().flex_wrap().gap_3().w_full();
        other = other
            .child(stat_pill(&format!(
                "Name servers: {}",
                join_or_none(&config.name_servers)
            )))
            .child(stat_pill(&format!(
                "{} ACLs · {} parsed rules",
                config.acls.len(),
                config.acls.iter().map(|acl| acl.rules.len()).sum::<usize>()
            )))
            .child(stat_pill(&format!(
                "{} DHCP pools",
                config.dhcp_pools.len()
            )));
        layout = layout.child(panel(
            "Other configuration",
            "Recognized feature blocks",
            other.into_any_element(),
        ));

        let mut acls = div().flex().flex_col().gap_2().w_full();
        for acl in &config.acls {
            acls = acls.child(config_list_row(vec![
                acl.name.clone(),
                format!("{} · {} rules", acl.acl_type, acl.rules.len()),
            ]));
            for rule in &acl.rules {
                acls = acls.child(code_line(rule.clone()));
            }
        }
        if config.acls.is_empty() {
            acls = acls.child(paragraph("No ACLs parsed."));
        }

        let mut pools = div().flex().flex_col().gap_2().w_full();
        for pool in &config.dhcp_pools {
            pools = pools.child(config_list_row(vec![
                pool.name.clone(),
                pool.network.clone(),
            ]));
            pools = pools.child(paragraph(format!(
                "Default router: {} · DNS: {}",
                pool.default_router
                    .clone()
                    .unwrap_or_else(|| "Not configured".into()),
                join_or_none(&pool.dns_servers)
            )));
        }
        if config.dhcp_pools.is_empty() {
            pools = pools.child(paragraph("No DHCP pools parsed."));
        }

        layout = layout.child(
            div()
                .flex()
                .flex_wrap()
                .gap_4()
                .w_full()
                .child(panel(
                    "Access control lists",
                    "Parsed ACL entries; unsupported syntax is reported below",
                    acls.into_any_element(),
                ))
                .child(panel(
                    "DHCP pools",
                    &format!("{} configured pools", config.dhcp_pools.len()),
                    pools.into_any_element(),
                )),
        );

        if !config.warnings.is_empty() {
            layout = layout.child(parser_warnings(&config.warnings));
        }

        // Backup / Restore panel
        let mut backup_panel = div().flex().flex_col().gap_3().w_full();
        backup_panel = backup_panel.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .w_full()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(palette::accent())
                                .child("Configuration Backup"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(palette::text_muted())
                                .child("Save and restore running-config backups"),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(Self::action_button(
                            if self.config.backups_loading {
                                "Backing up…"
                            } else {
                                "Backup running-config"
                            },
                            !self.config.backups_loading,
                            false,
                            cx,
                            ReefApp::backup_configuration,
                        ))
                        .child(Self::action_button(
                            if self.config.backups_loading {
                                "Loading…"
                            } else {
                                "Refresh"
                            },
                            !self.config.backups_loading,
                            false,
                            cx,
                            ReefApp::refresh_backups,
                        )),
                ),
        );

        if let Some(error) = &self.config.backups_error {
            backup_panel = backup_panel.child(error_panel(error));
        }

        // Backup list
        if !self.config.backups.is_empty() {
            let mut backup_list = div().flex().flex_col().gap_1().w_full();
            backup_list =
                backup_list.child(table_header(&["Created", "Source", "Size", "Actions"]));
            for backup in &self.config.backups {
                let backup_id = backup.id.clone();
                let size = format_bytes(backup.running_config.len() as u64);
                let source_label = match backup.source {
                    BackupSource::PreApply => "Pre-apply".to_string(),
                    BackupSource::Manual => "Manual".to_string(),
                    BackupSource::Scheduled => "Scheduled".to_string(),
                };
                backup_list = backup_list.child(table_row(
                    vec![
                        backup.created_at.clone(),
                        source_label,
                        size,
                        "".to_string(),
                    ],
                    true,
                ));
                // TODO: Make actions clickable
            }
            backup_panel = backup_panel.child(backup_list);
        } else {
            backup_panel = backup_panel.child(paragraph(
                "No backups yet. Create one to save the current running-config.",
            ));
        }

        layout = layout.child(panel(
            "Backup & Restore",
            "Save running-config backups and restore them later.",
            backup_panel.into_any_element(),
        ));

        layout
            .child(paragraph(
                "This is a read-only snapshot. Reef does not apply configuration changes from this screen. Secret-bearing lines are not returned to the UI.",
            ))
            .into_any_element()
    }

    fn configuration_topic_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(connected) = self.connected.as_ref() else {
            return empty_connected_panel(
                "Connect a device first",
                "This view requires an active device session.",
                cx,
            );
        };
        let name = connected.profile.name.clone();
        let active = self.active;
        let mut layout = div().flex().flex_col().gap_4().w_full().child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_4()
                .w_full()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(heading(match active {
                            Section::AccessLists => "Access control lists",
                            Section::Dhcp => "DHCP pools",
                            _ => "System settings",
                        }))
                        .child(paragraph(format!(
                            "{name} · structured configuration parsed from running configuration."
                        ))),
                )
                .child(Self::action_button(
                    if self.config.loading {
                        "Refreshing…"
                    } else {
                        "Refresh configuration"
                    },
                    !self.config.loading,
                    false,
                    cx,
                    ReefApp::refresh_configuration,
                )),
        );

        if let Some(error) = self.config.error.clone() {
            return layout
                .child(error_panel(&format!(
                    "Unable to retrieve configuration: {error}"
                )))
                .into_any_element();
        }
        if self.config.loading && self.config.data.is_none() {
            return layout
                .child(paragraph("Retrieving and parsing running configuration…"))
                .into_any_element();
        }
        let Some(config) = self.config.data.as_ref() else {
            return layout
                .child(paragraph("No configuration snapshot has been loaded yet."))
                .into_any_element();
        };

        match active {
            Section::AccessLists => {
                let mut content = div().flex().flex_col().gap_2().w_full();
                for acl in &config.acls {
                    content = content.child(config_list_row(vec![
                        acl.name.clone(),
                        format!("{} · {} rules", acl.acl_type, acl.rules.len()),
                    ]));
                    for rule in &acl.rules {
                        content = content.child(code_line(rule.clone()));
                    }
                }
                if config.acls.is_empty() {
                    content = content.child(paragraph(
                        "No access lists were recognized in the running configuration.",
                    ));
                }
                layout = layout.child(panel(
                    "Access control lists",
                    &format!(
                        "{} ACLs · {} parsed rules",
                        config.acls.len(),
                        config.acls.iter().map(|acl| acl.rules.len()).sum::<usize>()
                    ),
                    content.into_any_element(),
                ));
            }
            Section::Dhcp => {
                let mut content = div().flex().flex_col().gap_3().w_full();
                for pool in &config.dhcp_pools {
                    content = content.child(config_list_row(vec![
                        pool.name.clone(),
                        pool.network.clone(),
                    ]));
                    content = content.child(paragraph(format!(
                        "Default router: {} · DNS: {}",
                        pool.default_router
                            .clone()
                            .unwrap_or_else(|| "Not configured".into()),
                        join_or_none(&pool.dns_servers)
                    )));
                }
                if config.dhcp_pools.is_empty() {
                    content = content.child(paragraph(
                        "No DHCP pools were recognized in the running configuration.",
                    ));
                }
                layout = layout.child(panel(
                    "DHCP pools",
                    &format!("{} configured pools", config.dhcp_pools.len()),
                    content.into_any_element(),
                ));
            }
            _ => {
                layout = layout.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_3()
                        .w_full()
                        .child(stat_card(
                            "HOSTNAME",
                            config
                                .hostname
                                .clone()
                                .unwrap_or_else(|| "Not configured".into()),
                        ))
                        .child(stat_card(
                            "DOMAIN NAME",
                            config
                                .domain_name
                                .clone()
                                .unwrap_or_else(|| "Not configured".into()),
                        ))
                        .child(stat_card(
                            "NAME SERVERS",
                            config.name_servers.len().to_string(),
                        )),
                );
                let mut servers = div().flex().flex_col().gap_1().w_full();
                for server in &config.name_servers {
                    servers = servers.child(config_list_row(vec![server.clone()]));
                }
                if config.name_servers.is_empty() {
                    servers = servers.child(paragraph("No name servers are configured."));
                }
                layout = layout.child(panel(
                    "DNS resolvers",
                    "Configured IPv4 and IPv6 name servers",
                    servers.into_any_element(),
                ));
                if !config.warnings.is_empty() {
                    layout = layout.child(parser_warnings(&config.warnings));
                }
                layout = layout.child(paragraph(
                    "Read-only inspection. Credentials, line passwords, and other secret-bearing fields are not exposed in this view.",
                ));
            }
        }

        layout.into_any_element()
    }

    fn configuration_security_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(connected) = self.connected.as_ref() else {
            return empty_connected_panel(
                "Connect a device first",
                "This inspection requires an active device session.",
                cx,
            );
        };
        let name = connected.profile.name.clone();
        let active = self.active;
        let mut layout = div()
            .flex()
            .flex_col()
            .gap_4()
            .w_full()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .w_full()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(heading(if active == Section::Nat {
                                "Network address translation"
                            } else {
                                "SSH and line access"
                            }))
                            .child(paragraph(format!(
                                "{name} · read-only security and edge-network settings parsed from running configuration."
                            ))),
                    )
                    .child(Self::action_button(
                        if self.config.loading {
                            "Refreshing…"
                        } else {
                            "Refresh configuration"
                        },
                        !self.config.loading,
                        false,
                        cx,
                        ReefApp::refresh_configuration,
                    )),
            );

        if let Some(error) = self.config.error.clone() {
            return layout
                .child(error_panel(&format!(
                    "Unable to retrieve configuration: {error}"
                )))
                .into_any_element();
        }
        if self.config.loading && self.config.data.is_none() {
            return layout
                .child(paragraph("Retrieving and parsing running configuration…"))
                .into_any_element();
        }
        let Some(config) = self.config.data.as_ref() else {
            return layout
                .child(paragraph("No configuration snapshot has been loaded yet."))
                .into_any_element();
        };

        if active == Section::Nat {
            if let Some(nat) = config.nat.as_ref() {
                layout = layout.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_3()
                        .w_full()
                        .child(stat_card(
                            "INSIDE INTERFACES",
                            nat.inside_interfaces.len().to_string(),
                        ))
                        .child(stat_card(
                            "OUTSIDE INTERFACES",
                            nat.outside_interfaces.len().to_string(),
                        ))
                        .child(stat_card("ADDRESS POOLS", nat.pools.len().to_string()))
                        .child(stat_card("SOURCE RULES", nat.rules.len().to_string())),
                );

                let mut inside = div().flex().flex_col().gap_1().w_full();
                for name in &nat.inside_interfaces {
                    inside = inside.child(config_list_row(vec![name.clone()]));
                }
                if nat.inside_interfaces.is_empty() {
                    inside = inside.child(paragraph("No inside interfaces parsed."));
                }
                let mut outside = div().flex().flex_col().gap_1().w_full();
                for name in &nat.outside_interfaces {
                    outside = outside.child(config_list_row(vec![name.clone()]));
                }
                if nat.outside_interfaces.is_empty() {
                    outside = outside.child(paragraph("No outside interfaces parsed."));
                }
                layout = layout.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_4()
                        .w_full()
                        .child(panel(
                            "Inside interfaces",
                            "Interfaces marked for NAT translation",
                            inside.into_any_element(),
                        ))
                        .child(panel(
                            "Outside interfaces",
                            "Interfaces facing translated networks",
                            outside.into_any_element(),
                        )),
                );

                let mut pools = div().flex().flex_col().gap_1().w_full();
                for pool in &nat.pools {
                    pools = pools.child(config_list_row(vec![
                        pool.name.clone(),
                        format!("{} – {}", pool.start, pool.end),
                        format!("Mask {}", pool.netmask),
                    ]));
                }
                if nat.pools.is_empty() {
                    pools = pools.child(paragraph("No NAT pools parsed."));
                }
                layout = layout.child(panel(
                    "NAT pools",
                    "Configured public address ranges",
                    pools.into_any_element(),
                ));

                let mut rules = div().flex().flex_col().gap_1().w_full();
                for rule in &nat.rules {
                    rules = rules.child(config_list_row(vec![
                        rule.name.clone(),
                        format!(
                            "ACL {} · Pool {} · {}",
                            rule.source_acl.as_deref().unwrap_or("—"),
                            rule.pool.as_deref().unwrap_or("—"),
                            if rule.overload {
                                "PAT / overload"
                            } else {
                                "No overload"
                            }
                        ),
                    ]));
                }
                if nat.rules.is_empty() {
                    rules = rules.child(paragraph("No supported dynamic NAT rules parsed."));
                }
                layout = layout.child(panel(
                    "Translation rules",
                    "Parsed dynamic source NAT rules",
                    rules.into_any_element(),
                ));
            } else {
                layout = layout.child(paragraph(
                    "No supported NAT configuration was recognized. Static NAT, route-map NAT, and other unsupported forms are not inferred.",
                ));
            }
        } else {
            let ssh = config.ssh.as_ref();
            layout = layout.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_3()
                    .w_full()
                    .child(stat_card(
                        "SSH CONFIGURATION",
                        ssh.map(|ssh| if ssh.enabled { "Enabled" } else { "Disabled" })
                            .unwrap_or("Not parsed")
                            .to_string(),
                    ))
                    .child(stat_card(
                        "SSH VERSION",
                        ssh.map(|ssh| ssh.version.clone())
                            .unwrap_or_else(|| "Unknown".into()),
                    ))
                    .child(stat_card(
                        "SSH TIMEOUT",
                        ssh.and_then(|ssh| ssh.timeout)
                            .map(|timeout| format!("{timeout}s"))
                            .unwrap_or_else(|| "Default / unknown".into()),
                    ))
                    .child(stat_card(
                        "AUTH RETRIES",
                        ssh.and_then(|ssh| ssh.authentication_retries)
                            .map(|retries| retries.to_string())
                            .unwrap_or_else(|| "Default / unknown".into()),
                    )),
            );

            let mut lines = div().flex().flex_col().gap_1().w_full();
            for line in &config.line_access {
                lines = lines.child(config_list_row(vec![
                    line.line.clone(),
                    if line.login_required {
                        "Login required".to_string()
                    } else {
                        "Login not required / unknown".to_string()
                    },
                    format!(
                        "Transport: {}",
                        if line.transport.is_empty() {
                            "Not configured".to_string()
                        } else {
                            line.transport.join(", ")
                        }
                    ),
                ]));
            }
            if config.line_access.is_empty() {
                lines = lines.child(paragraph("No line access configuration parsed."));
            }
            layout = layout.child(panel(
                "Line access policy",
                "Console and VTY transport/login settings; no passwords or secrets are displayed",
                lines.into_any_element(),
            ));
            layout = layout.child(paragraph(
                "This view reports configuration syntax only. It does not verify that SSH is reachable, that keys exist, or that AAA authentication succeeds.",
            ));

            // SSH Host Key Management panel
            layout = layout.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .w_full()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(palette::accent())
                                    .child("SSH Host Key Management"),
                            )
                            .child(
                                div().text_xs().text_color(palette::text_muted()).child(
                                    "Enroll and manage trusted SSH host keys in known_hosts.",
                                ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(Self::action_button(
                                if self.config.known_hosts_loading {
                                    "Loading…"
                                } else {
                                    "Refresh host keys"
                                },
                                !self.config.known_hosts_loading,
                                false,
                                cx,
                                ReefApp::refresh_known_hosts,
                            ))
                            .child(Self::action_button(
                                if self.config.known_hosts_show_enroll {
                                    "Cancel enroll"
                                } else {
                                    "Enroll host key"
                                },
                                !self.config.known_hosts_loading,
                                false,
                                cx,
                                |this, cx| {
                                    this.config.known_hosts_show_enroll =
                                        !this.config.known_hosts_show_enroll;
                                    cx.notify();
                                },
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .w_full()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .w_full()
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .w_full()
                                            .child(Self::field_block(
                                                "Host",
                                                self.config.enroll_host.as_ref().unwrap().clone(),
                                            ))
                                            .child(Self::field_block(
                                                "Key type",
                                                self.config
                                                    .enroll_key_type
                                                    .as_ref()
                                                    .unwrap()
                                                    .clone(),
                                            ))
                                            .child(Self::field_block(
                                                "Key hex",
                                                self.config
                                                    .enroll_key_hex
                                                    .as_ref()
                                                    .unwrap()
                                                    .clone(),
                                            )),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .justify_end()
                                            .gap_2()
                                            .child(Self::action_button(
                                                "Cancel",
                                                true,
                                                false,
                                                cx,
                                                |this, cx| {
                                                    this.config.known_hosts_show_enroll = false;
                                                    cx.notify();
                                                },
                                            ))
                                            .child(Self::action_button(
                                                "Enroll",
                                                !self
                                                    .config
                                                    .enroll_host
                                                    .as_ref()
                                                    .unwrap()
                                                    .read(cx)
                                                    .is_empty()
                                                    && !self
                                                        .config
                                                        .enroll_key_type
                                                        .as_ref()
                                                        .unwrap()
                                                        .read(cx)
                                                        .is_empty()
                                                    && !self
                                                        .config
                                                        .enroll_key_hex
                                                        .as_ref()
                                                        .unwrap()
                                                        .read(cx)
                                                        .is_empty()
                                                    && !self.config.known_hosts_loading,
                                                false,
                                                cx,
                                                ReefApp::enroll_host_key_submit,
                                            )),
                                    ),
                            )
                            .into_any_element(),
                    )
                    .into_any_element(),
            );

            if let Some(error) = &self.config.known_hosts_error {
                layout = layout.child(error_panel(error));
            }

            // Known hosts list
            if !self.config.known_hosts.is_empty() {
                let mut host_list = div().flex().flex_col().gap_1().w_full();
                host_list = host_list.child(table_header(&[
                    "Host",
                    "Key type",
                    "Key (SHA256 fingerprint)",
                    "Actions",
                ]));
                for entry in &self.config.known_hosts {
                    let host = entry.host.clone();
                    let fingerprint = if entry.key_hex.len() > 32 {
                        format!(
                            "{}…{}",
                            &entry.key_hex[..16],
                            &entry.key_hex[entry.key_hex.len() - 16..]
                        )
                    } else {
                        entry.key_hex.clone()
                    };
                    host_list = host_list.child(table_row(
                        vec![
                            entry.host.clone(),
                            entry.key_type.clone(),
                            fingerprint,
                            "".to_string(),
                        ],
                        false,
                    ));
                }
                layout = layout.child(panel(
                    "Known host keys",
                    "Trusted SSH host keys stored in known_hosts.",
                    host_list.into_any_element(),
                ));
            } else {
                layout = layout.child(paragraph(
                    "No known host keys enrolled yet. Connect to an SSH device with Accept/Warn policy, or manually enroll a host key above.",
                ));
            }
        }

        if !config.warnings.is_empty() {
            layout = layout.child(parser_warnings(&config.warnings));
        }
        layout
            .child(paragraph(
                "Read-only inspection. No configuration commands are sent to the device.",
            ))
            .into_any_element()
    }

    // ------------------------------------------------------------------
    // Terminal
    // ------------------------------------------------------------------

    fn terminal_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(connected) = self.connected.as_ref() else {
            return empty_connected_panel(
                "Connect a device first",
                "The terminal becomes available after a device session is established.",
                cx,
            );
        };
        let name = connected.profile.name.clone();
        let host = connected.profile.host.clone();
        let transport = transport_label(&connected.profile.transport);
        let recording_active = self
            .terminal
            .recording
            .as_ref()
            .is_some_and(|status| status.active);
        let recording_summary = self.terminal.recording.as_ref().map(|status| {
            if status.active {
                format!(
                    "REC · {} · {} part{}{}",
                    format_bytes(status.bytes),
                    status.parts,
                    if status.parts == 1 { "" } else { "s" },
                    if status.redactions > 0 {
                        format!(" · {} redaction(s)", status.redactions)
                    } else {
                        String::new()
                    }
                )
            } else {
                format!(
                    "Saved · {}",
                    status.path.as_deref().unwrap_or("no recording path")
                )
            }
        });
        let busy = self.terminal.busy;
        let recording_busy = self.terminal.recording_busy;
        let command = self.terminal.command.clone();
        let output = self.terminal.output.clone();

        let mut toolbar = div().flex().flex_wrap().items_center().gap_2().w_full();
        toolbar = toolbar
            .child(Self::action_button(
                if recording_busy {
                    "Working…"
                } else if recording_active {
                    "Stop recording"
                } else {
                    "Start recording"
                },
                !recording_busy,
                false,
                cx,
                ReefApp::toggle_recording,
            ))
            .child(Self::action_button(
                "Ctrl+C",
                true,
                false,
                cx,
                ReefApp::interrupt_terminal,
            ))
            .child(Self::action_button(
                "Clear view",
                true,
                false,
                cx,
                ReefApp::clear_terminal,
            ))
            .child(Self::action_button(
                "Copy",
                true,
                false,
                cx,
                ReefApp::copy_terminal,
            ))
            .child(Self::action_button(
                "Export logs",
                !self.diagnostics_ui.busy,
                false,
                cx,
                ReefApp::export_diagnostics,
            ))
            .child(Self::action_button(
                "Open log folder",
                !self.diagnostics_ui.busy,
                false,
                cx,
                ReefApp::open_logs_directory,
            ))
            .child(Self::action_button(
                "Diagnostics",
                true,
                false,
                cx,
                |this, cx| this.navigate(Section::Diagnostics, cx),
            ))
            .child(Self::action_button(
                "History",
                !self.terminal.command_history.is_empty(),
                false,
                cx,
                |this, cx| {
                    if let Some(last) = this.terminal.command_history.last() {
                        this.terminal.command.update(cx, |f, cx| {
                            f.set_content(last.clone(), cx);
                        });
                    }
                    cx.notify();
                },
            ));
        if let Some(summary) = recording_summary {
            toolbar = toolbar.child(
                div()
                    .ml_auto()
                    .text_xs()
                    .text_color(if recording_active {
                        palette::danger()
                    } else {
                        palette::text_muted()
                    })
                    .child(summary),
            );
        }

        let mut layout = div()
            .flex()
            .flex_col()
            .gap_3()
            .w_full()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .w_full()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(heading(name))
                            .child(paragraph(format!("{host} · {transport} · command session"))),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_xs()
                            .text_color(palette::accent_strong())
                            .child(div().size(px(8.)).rounded_full().bg(palette::accent()))
                            .child("Session active"),
                    ),
            )
            .child(toolbar)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .text_xs()
                    .text_color(palette::text_faint())
                    .w_full()
                    .child(div().text_color(palette::text_faint()).child("Shortcuts:"))
                    .child(div().text_color(palette::text_faint()).child("↑/↓ History"))
                    .child(
                        div()
                            .text_color(palette::text_faint())
                            .child("Ctrl+C Interrupt"),
                    )
                    .child(div().text_color(palette::text_faint()).child("Enter Send")),
            );

        if let Some(error) = self.terminal.error.clone() {
            layout = layout.child(error_panel(&error));
        } else if let Some(message) = self.terminal.message.clone() {
            layout = layout.child(notice_panel(message));
        }

        layout = layout
            .child(
                div()
                    .id("terminal-output")
                    .w_full()
                    .h(px(420.))
                    .overflow_y_scroll()
                    .track_scroll(&self.terminal.scroll)
                    .p_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(palette::panel_border())
                    .bg(palette::terminal_bg())
                    .child(
                        div()
                            .w_full()
                            .font_family(".SystemMonospaceFont")
                            .text_size(px(13.))
                            .line_height(px(19.))
                            .text_color(palette::text())
                            .whitespace_normal()
                            .child(output),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .w_full()
                    .child(
                        div()
                            .text_color(palette::accent())
                            .font_family(".SystemMonospaceFont")
                            .child("$"),
                    )
                    .child(div().flex_1().child(command))
                    .child(Self::action_button(
                        if busy { "Running…" } else { "Send" },
                        !busy,
                        true,
                        cx,
                        |this, cx| {
                            let text = this.terminal.command.read(cx).text();
                            this.submit_terminal_command(text, cx);
                            this.terminal
                                .command
                                .update(cx, |field, cx| field.clear(cx));
                        },
                    )),
            )
            .child(paragraph(
                "Commands execute on the connected device. Output is returned after each command completes. Recording captures the full backend response; the visible terminal may truncate very large output.",
            ));

        layout.into_any_element()
    }

    // ------------------------------------------------------------------
    // Diagnostics
    // ------------------------------------------------------------------

    fn diagnostics_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mut layout = div()
            .flex()
            .flex_col()
            .gap_4()
            .w_full()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .w_full()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(heading("Diagnostics and session logs"))
                            .child(paragraph(
                                "Portable logs live beside the Reef executable. Passwords, private keys, and tokens are redacted before writing.",
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .child(Self::action_button(
                                if self.diagnostics_ui.loading || self.diagnostics_ui.files_loading {
                                    "Refreshing…"
                                } else {
                                    "Refresh"
                                },
                                !(self.diagnostics_ui.loading
                                    || self.diagnostics_ui.files_loading),
                                false,
                                cx,
                                |this, cx| {
                                    this.refresh_diagnostics(cx);
                                    this.refresh_log_files(cx);
                                },
                            ))
                            .child(Self::action_button(
                                "Open log folder",
                                !self.diagnostics_ui.busy,
                                false,
                                cx,
                                ReefApp::open_logs_directory,
                            ))
                            .child(Self::action_button(
                                "Export ZIP",
                                !self.diagnostics_ui.busy,
                                true,
                                cx,
                                ReefApp::export_diagnostics,
                            )),
                    ),
            );

        if let Some(error) = self.diagnostics_ui.error.clone() {
            layout = layout.child(error_panel(&error));
        }
        if let Some(message) = self.diagnostics_ui.message.clone() {
            layout = layout.child(notice_panel(message));
        }

        let Some(status) = self.diagnostics_ui.status.clone() else {
            if self.diagnostics_ui.loading {
                layout = layout.child(paragraph("Loading diagnostics…"));
            } else {
                layout = layout.child(paragraph(
                    "Refresh to capture the current diagnostics status and log file list.",
                ));
            }
            return layout.into_any_element();
        };

        layout = layout.child(
            div()
                .flex()
                .flex_wrap()
                .gap_3()
                .w_full()
                .child(stat_card(
                    "RUN ID",
                    status.run_id.chars().take(8).collect::<String>(),
                ))
                .child(stat_card("LEVEL", status.level.clone()))
                .child(stat_card("UPTIME", format_uptime(status.uptime_seconds)))
                .child(stat_card(
                    "PORTABLE",
                    if status.portable { "Yes" } else { "Not marked" }.to_string(),
                )),
        );

        let mut environment = div().flex().flex_wrap().gap_3().w_full();
        environment = environment
            .child(info_cell(
                "App version",
                format!(
                    "{}{}",
                    status.environment.app_version,
                    if status.environment.debug_build {
                        " (debug)"
                    } else {
                        ""
                    }
                ),
            ))
            .child(info_cell(
                "OS version",
                status.environment.os_version.clone(),
            ))
            .child(info_cell(
                "Kernel",
                status.environment.kernel_version.clone(),
            ))
            .child(info_cell("Host", status.environment.host_name.clone()))
            .child(info_cell(
                "Logical CPUs",
                status.environment.logical_cpus.to_string(),
            ))
            .child(info_cell(
                "Total memory",
                format_bytes(status.environment.total_memory_bytes),
            ))
            .child(info_cell(
                "Executable",
                status.environment.executable.clone(),
            ))
            .child(info_cell("Portable root", status.root.clone()));

        let mut environment_body = div().flex().flex_col().gap_3().w_full().child(environment);
        if !status.writable_failures.is_empty() {
            let mut failures = div().flex().flex_col().gap_1().w_full();
            for failure in &status.writable_failures {
                failures = failures.child(paragraph(failure.clone()));
            }
            environment_body = environment_body.child(error_panel_with_body(
                "Some portable directories are not writable",
                failures.into_any_element(),
            ));
        }
        if let Some(degraded) = status.degraded.as_ref() {
            environment_body = environment_body.child(error_panel(&format!(
                "Diagnostic engine degraded: {degraded}"
            )));
        }
        layout = layout.child(panel(
            "Environment",
            "Captured at startup and included in every diagnostic export.",
            environment_body.into_any_element(),
        ));

        let mut files = div().flex().flex_col().gap_2().w_full();
        if let Some(error) = self.diagnostics_ui.files_error.clone() {
            files = files.child(error_panel(&error));
        } else if self.diagnostics_ui.files.is_empty() {
            files = files.child(paragraph(
                "No log files yet. Connect to a device to create a session recording.",
            ));
        } else {
            files = files.child(table_header(&["File", "Kind", "Size"]));
            let count = self.diagnostics_ui.files.len();
            for index in 0..count {
                let file = &self.diagnostics_ui.files[index];
                let relative = file.relative_path.clone();
                let kind = file.kind.clone();
                let size = format_bytes(file.size_bytes);
                let row = table_row(vec![relative, kind, size], false);
                files = files.child(
                    div()
                        .id(("log-row", index))
                        .flex()
                        .items_center()
                        .gap_2()
                        .w_full()
                        .child(row)
                        .child(Self::action_button(
                            "View",
                            true,
                            false,
                            cx,
                            move |this, cx| this.view_log(index, cx),
                        )),
                );
            }
        }
        layout = layout.child(panel(
            "Log files",
            &format!(
                "{} files under {}",
                self.diagnostics_ui.files.len(),
                status.logs_directory
            ),
            files.into_any_element(),
        ));

        if let Some(error) = self.diagnostics_ui.view_error.clone() {
            layout = layout.child(error_panel(&error));
        }
        if let Some(log) = self.diagnostics_ui.selected.as_ref() {
            let path = log.path.clone();
            let meta = format!(
                "{} · {}{}",
                path,
                format_bytes(log.size_bytes),
                if log.truncated {
                    " · preview truncated at 2 MiB"
                } else {
                    ""
                }
            );
            let content = log.content.clone();
            layout = layout.child(panel(
                "Log preview",
                &meta,
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .w_full()
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .justify_end()
                            .child(Self::action_button(
                                "Copy",
                                true,
                                false,
                                cx,
                                ReefApp::copy_log,
                            ))
                            .child(Self::action_button("Close", true, false, cx, |this, cx| {
                                this.diagnostics_ui.selected = None;
                                cx.notify();
                            })),
                    )
                    .child(code_block("log-content", content))
                    .into_any_element(),
            ));
        }

        layout = layout.child(panel(
            "Classroom test note",
            "Notes are appended to Logs/TestNotes/test-notes.md and included in exports.",
            div()
                .flex()
                .flex_col()
                .gap_3()
                .w_full()
                .child(self.diagnostics_ui.note.clone())
                .child(div().flex().justify_end().child(Self::action_button(
                    "Save test note",
                    !self.diagnostics_ui.busy && !self.diagnostics_ui.note.read(cx).is_empty(),
                    true,
                    cx,
                    ReefApp::save_test_note,
                )))
                .into_any_element(),
        ));

        layout
            .child(paragraph(
                "Terminal recordings default to on and are stored under Logs/Sessions. The recorder keeps full backend output even when the visible terminal is truncated or cleared. Startup-configuration writes are disabled and logged without secrets.",
            ))
            .into_any_element()
    }
}

impl Render for ReefApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Register global Escape handler once so it dismisses the active modal
        // even when a text field or other element has focus.
        if !self.escape_bound {
            cx.on_action(
                TypeId::of::<DismissModal>(),
                window,
                |this, _action, _phase, _window, cx| {
                    let _ = _window;
                    this.dismiss_modal(cx);
                },
            );
            self.escape_bound = true;
        }

        let body = self.section_body(cx);
        let sidebar = self.sidebar(cx);
        let topbar = self.topbar(cx);
        let heading = self.page_heading(window, cx);

        let mut toasts_content = div().flex().flex_col().gap_2().w_full();
        for toast in &self.toasts {
            let color = toast.kind.color();
            let id = toast.id;
            toasts_content = toasts_content.child(
                div()
                    .id(("toast", id))
                    .flex()
                    .items_start()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .border_1()
                    .border_color(color)
                    .bg(palette::panel())
                    .text_color(palette::text())
                    .text_sm()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .text_color(color)
                            .child(toast.kind.label()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette::text_muted())
                            .child(toast.message.clone()),
                    )
                    .child(
                        div()
                            .size(px(16.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hover(|style| style.bg(palette::panel_alt()))
                            .child("✕"),
                    ),
            );
        }

        let toasts = div()
            .absolute()
            .top(px(20.))
            .right(px(20.))
            .flex()
            .flex_col()
            .gap_2()
            .child(toasts_content);

        let mut content = div()
            .size_full()
            .flex()
            .flex_row()
            .bg(palette::background())
            .text_color(palette::text())
            .font_family(".SystemUIFont")
            .child(sidebar)
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(topbar)
                    .child(
                        div()
                            .id("reef-content")
                            .flex_1()
                            .min_h(px(0.))
                            .overflow_y_scroll()
                            .track_scroll(&self.content_scroll)
                            .p_6()
                            .flex()
                            .flex_col()
                            .gap_4()
                            .child(heading)
                            .child(safety_banner())
                            .child(body),
                    ),
            );

        // Modal backdrop + dialog rendered at root, above all content.
        if let Some(modal) = &self.modal {
            let dirty = self.modal_is_dirty(cx);
            let error = self.modal_error.clone();
            let pending = self.pending_modal.clone();
            let content_scroll = &self.content_scroll;
            let modal_panel = self.modal_panel(
                modal.clone(),
                dirty,
                error.as_deref(),
                pending.is_some(),
                window,
                cx,
            );
            let weak = cx.entity().downgrade();
            content = content.child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(palette::backdrop())
                    .occlude()
                    .on_mouse_down_out(move |event, window, app_cx| {
                        let _ = event;
                        let _ = window;
                        weak.update(app_cx, |this, cx| {
                            this.request_modal_close(cx);
                        })
                        .ok();
                    })
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .p_4()
                            .child(modal_panel),
                    ),
            );
        }

        content.child(toasts)
    }
}

impl ReefApp {
    fn modal_panel(
        &mut self,
        modal: Modal,
        dirty: bool,
        error: Option<&str>,
        has_pending: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match modal {
            Modal::Connection => self.connection_panel(window, cx),
            Modal::Profile => self.profile_panel(window, cx),
            Modal::Interface => self.interface_panel(window, cx),
            Modal::Vlan => self.vlan_panel(window, cx),
            Modal::Route => self.route_panel(window, cx),
            Modal::SystemSettings => self.system_settings_panel(window, cx),
            Modal::ConfirmDiscard => self.confirm_discard_panel(window, cx),
            Modal::ConfirmApply => self.confirm_apply_panel(window, cx),
            Modal::ConfirmRestore => self.confirm_restore_panel(window, cx),
        };
        let _ = (dirty, error, has_pending, window);
        div().into_any_element()
    }

    fn connection_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(form) = self.connection.as_mut() else {
            return div().into_any_element();
        };
        let profile = form.profile.clone();
        let is_serial = profile.transport == ProfileTransport::Serial;
        let error = form.error.clone();
        let connecting = form.connecting;
        let trust = form.trust_host_key;
        let password = form.password.clone();
        let enable_secret = form.enable_secret.clone();

        let mut auth = div().flex().flex_col().gap_3().w_full();
        if !is_serial {
            auth = auth
                .child(Self::field_block("Password", password))
                .child(Self::field_block("Enable secret (optional)", enable_secret));
        }
        if let Some(error) = error {
            let unknown_key = error.to_lowercase().contains("unknown host key");
            auth = auth.child(error_panel(&error));
            if unknown_key && profile.transport == ProfileTransport::Ssh {
                auth = auth.child(checkbox_row(
                    "Trust this SSH host key and save it for future connections",
                    trust,
                    cx,
                ));
            }
        }
        auth = auth
            .child(
                div()
                    .flex()
                    .gap_2()
                    .justify_end()
                    .child(Self::action_button("Cancel", true, false, cx, |this, cx| {
                        this.connection = None;
                        this.close_modal();
                        cx.notify();
                    }))
                    .child(Self::action_button(
                        if connecting { "Connecting…" } else { "Connect" },
                        !connecting,
                        true,
                        cx,
                        ReefApp::connect_selected,
                    )),
            )
            .child(paragraph(
                "Credentials are sent to the Rust session service for this connection only. They are not written to the profile.",
            ));

        self.modal_shell(
            format!("Connect to {}", profile.name).into(),
            format!("{} · {}", profile.host, transport_label(&profile.transport)).into(),
            auth.into_any_element(),
            vec![
                (
                    "Cancel".into(),
                    false,
                    false,
                    Box::new(cx.listener(|this, _, _, cx| {
                        this.connection = None;
                        this.close_modal();
                        cx.notify();
                    })),
                ),
                (
                    if connecting {
                        "Connecting…"
                    } else {
                        "Connect"
                    }
                    .into(),
                    !connecting,
                    true,
                    Box::new(cx.listener(
                        |this: &mut ReefApp,
                         _: &MouseDownEvent,
                         _: &mut Window,
                         cx: &mut Context<ReefApp>| {
                            ReefApp::connect_selected(this, cx);
                        },
                    )),
                ),
            ],
            cx,
        )
    }

    fn profile_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let form = &self.profile_form;
        let is_serial = form.transport == TransportChoice::Serial;
        let error = form.error.clone();
        let saving = form.saving;

        let mut transport_buttons = div().flex().gap_2().w_full();
        for choice in [
            TransportChoice::Ssh,
            TransportChoice::Telnet,
            TransportChoice::Serial,
        ] {
            let active = form.transport == choice;
            let mut button = div()
                .flex()
                .items_center()
                .justify_center()
                .h(px(32.))
                .px_3()
                .rounded_md()
                .text_xs()
                .cursor_pointer()
                .child(choice.label().to_string());
            if active {
                button = button
                    .bg(palette::accent_dim())
                    .text_color(palette::accent_strong())
                    .font_weight(FontWeight::SEMIBOLD);
            } else {
                button = button
                    .bg(palette::panel_alt())
                    .text_color(palette::text_muted())
                    .border_1()
                    .border_color(palette::panel_border());
            }
            transport_buttons = transport_buttons.child(
                button
                    .id(ElementId::Name(choice.label().into()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select_transport(choice, cx);
                    })),
            );
        }

        let mut new_profile = div()
            .flex()
            .flex_col()
            .gap_3()
            .w_full()
            .child(Self::field_block("Profile name", form.name.clone()))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .w_full()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(palette::text_muted())
                            .child("Transport"),
                    )
                    .child(transport_buttons),
            )
            .child(Self::field_block(
                if form.transport.is_serial() {
                    "Serial port"
                } else {
                    "Host or IP address"
                },
                form.host.clone(),
            ));

        if !form.transport.is_serial() {
            new_profile = new_profile
                .child(Self::field_block("Port", form.port.clone()))
                .child(Self::field_block("Username", form.username.clone()));
        }
        if let Some(error) = error {
            new_profile = new_profile.child(error_panel(&error));
        }
        new_profile = new_profile.child(div().flex().justify_end().child(Self::action_button(
            if saving { "Saving…" } else { "Save profile" },
            !saving,
            true,
            cx,
            ReefApp::save_profile,
        )));

        self.modal_shell(
            "New connection profile".into(),
            "Save a reusable device profile. Authentication secrets are requested separately when connecting.".into(),
            new_profile.into_any_element(),
            vec![
                ("Cancel".into(), false, false, Box::new(cx.listener(|this, _, _, cx| {
                    this.close_modal();
                    cx.notify();
                }))),
                (
                    "Save profile".into(),
                    !saving,
                    true,
                    Box::new(cx.listener(|this: &mut ReefApp, _: &MouseDownEvent, _: &mut Window, cx: &mut Context<ReefApp>| {
                        ReefApp::save_profile(this, cx);
                    }))),
            ],
            cx,
        )
    }

    fn interface_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(form) = self.interface_edit.as_mut() else {
            return div().into_any_element();
        };
        let editing = form.editing;
        let enabled = form.enabled;
        let mode = form.mode;
        let title = if editing {
            "Edit interface".to_string()
        } else {
            "Add interface".to_string()
        };
        let subtitle = if editing {
            "Change the interface's configuration in the preview.".to_string()
        } else {
            "Enter interface settings to add to the preview.".to_string()
        };
        let add_btn_label = if self.config.preview_loading {
            "Applying…"
        } else if editing {
            "Save changes"
        } else {
            "Add interface"
        };
        let name = field_owned(
            cx,
            "GigabitEthernet1/0/1",
            false,
            &form.name.read(cx).text(),
        );
        let description = field_owned(
            cx,
            "Interface description",
            false,
            &form.description.read(cx).text(),
        );
        let access_vlan = field_owned(cx, "10", false, &form.access_vlan.read(cx).text());
        let ipv4_address = field_owned(cx, "10.0.0.1", false, &form.ipv4_address.read(cx).text());
        let ipv4_prefix_len = field_owned(cx, "24", false, &form.ipv4_prefix_len.read(cx).text());
        // Drop the mutable borrow so self can be borrowed again for modal_shell
        let _ = form;

        let mut mode_pills = div().flex().gap_1().w_full();
        for (choice, label) in [
            (InterfaceModeChoice::Routed, "Routed (L3)"),
            (InterfaceModeChoice::Access, "Access (L2)"),
            (InterfaceModeChoice::Trunk, "Trunk"),
            (InterfaceModeChoice::L2, "L2 (switch)"),
        ] {
            let active = mode == choice;
            let mut pill = div()
                .flex()
                .items_center()
                .justify_center()
                .px_2()
                .py_1()
                .rounded_md()
                .cursor_pointer()
                .text_xs()
                .child(label.to_string());
            if active {
                pill = pill
                    .bg(palette::selection())
                    .border_1()
                    .border_color(palette::accent_dim());
            } else {
                pill = pill
                    .bg(palette::panel())
                    .border_1()
                    .border_color(palette::panel_border())
                    .text_color(palette::text_muted());
            }
            mode_pills = mode_pills.child(pill.on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    if let Some(f) = this.interface_edit.as_mut() {
                        f.mode = choice;
                    }
                    cx.notify();
                }),
            ));
        }

        let body = div()
            .flex()
            .flex_col()
            .gap_3()
            .w_full()
            .child(Self::field_block("Name", name.clone()))
            .child(Self::field_block("Description", description.clone()))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .w_full()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(palette::text_muted())
                            .child("Mode"),
                    )
                    .child(mode_pills),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .w_full()
                    .child(ui::toggle(
                        "iface-enabled",
                        enabled,
                        "Administratively enabled",
                        cx.listener(move |this, _, _, cx| {
                            if let Some(f) = this.interface_edit.as_mut() {
                                f.enabled = !f.enabled;
                            }
                            cx.notify();
                        }),
                    ))
                    .child(div().flex_1()),
            )
            .child(Self::field_block("Access VLAN ID", access_vlan.clone()))
            .child(Self::field_block("IPv4 address", ipv4_address.clone()))
            .child(Self::field_block("Prefix length", ipv4_prefix_len.clone()))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(Self::action_button(
                        "Cancel",
                        true,
                        false,
                        cx,
                        |this, cx| {
                            this.interface_edit = None;
                            this.close_modal();
                            cx.notify();
                        },
                    ))
                    .child(Self::action_button(
                        add_btn_label.clone(),
                        !name.read(cx).is_empty() && !self.config.preview_loading,
                        true,
                        cx,
                        ReefApp::edit_interface_submit,
                    )),
            );

        self.modal_shell(
            title.into(),
            subtitle.into(),
            body.into_any_element(),
            vec![
                (
                    "Cancel".into(),
                    true,
                    false,
                    Box::new(cx.listener(|this, _, _, cx| {
                        this.interface_edit = None;
                        this.close_modal();
                        cx.notify();
                    })),
                ),
                (
                    add_btn_label.into(),
                    !name.read(cx).is_empty() && !self.config.preview_loading,
                    true,
                    Box::new(cx.listener(
                        |this: &mut ReefApp,
                         _: &MouseDownEvent,
                         _: &mut Window,
                         cx: &mut Context<ReefApp>| {
                            ReefApp::edit_interface_submit(this, cx);
                        },
                    )),
                ),
            ],
            cx,
        )
    }

    fn vlan_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(form) = self.vlan_edit.as_mut() else {
            return div().into_any_element();
        };
        let editing = form.editing;
        let enabled = form.enabled;
        let title = if editing {
            "Edit VLAN".to_string()
        } else {
            "Add VLAN".to_string()
        };
        let subtitle = if editing {
            "Change the VLAN's name or administrative state.".to_string()
        } else {
            "Enter VLAN settings to add to the preview.".to_string()
        };
        let vid = form.id.read(cx).text();
        let vname = form.name.read(cx).text();
        let _ = form;

        let body = div()
            .flex()
            .flex_col()
            .gap_3()
            .w_full()
            .child(Self::field_block(
                "VLAN ID",
                field_owned(cx, "10", false, &vid),
            ))
            .child(Self::field_block(
                "VLAN name",
                field_owned(cx, "USERS", false, &vname),
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .w_full()
                    .child(ui::toggle(
                        "vlan-enabled",
                        enabled,
                        "VLAN enabled",
                        cx.listener(move |this, _, _, cx| {
                            if let Some(f) = this.vlan_edit.as_mut() {
                                f.enabled = !f.enabled;
                            }
                            cx.notify();
                        }),
                    ))
                    .child(div().flex_1()),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(Self::action_button(
                        "Cancel",
                        true,
                        false,
                        cx,
                        |this, cx| {
                            this.vlan_edit = None;
                            this.close_modal();
                            cx.notify();
                        },
                    ))
                    .child(Self::action_button(
                        if self.config.preview_loading {
                            "Applying…"
                        } else {
                            "Save VLAN"
                        },
                        !vid.is_empty() && !vname.is_empty() && !self.config.preview_loading,
                        true,
                        cx,
                        ReefApp::edit_vlan_submit,
                    )),
            );

        self.modal_shell(
            title.into(),
            subtitle.into(),
            body.into_any_element(),
            vec![
                (
                    "Cancel".into(),
                    true,
                    false,
                    Box::new(cx.listener(|this, _, _, cx| {
                        this.vlan_edit = None;
                        this.close_modal();
                        cx.notify();
                    })),
                ),
                (
                    "Save VLAN".into(),
                    !vid.is_empty() && !vname.is_empty() && !self.config.preview_loading,
                    true,
                    Box::new(cx.listener(
                        |this: &mut ReefApp,
                         _: &MouseDownEvent,
                         _: &mut Window,
                         cx: &mut Context<ReefApp>| {
                            ReefApp::edit_vlan_submit(this, cx);
                        },
                    )),
                ),
            ],
            cx,
        )
    }

    fn route_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(form) = self.route_edit.as_mut() else {
            return div().into_any_element();
        };
        let editing = form.editing;
        let title = if editing {
            "Edit route".to_string()
        } else {
            "Add static route".to_string()
        };
        let subtitle = if editing {
            "Replace the route's next hop or administrative distance.".to_string()
        } else {
            "Enter route settings to add to the preview.".to_string()
        };
        let prefix = form.prefix.read(cx).text();
        let next_hop = form.next_hop.read(cx).text();
        let admin_distance = form.admin_distance.read(cx).text();
        let _ = form;

        let body = div()
            .flex()
            .flex_col()
            .gap_3()
            .w_full()
            .child(Self::field_block(
                "Prefix (CIDR)",
                field_owned(cx, "192.0.2.0/24", false, &prefix),
            ))
            .child(Self::field_block(
                "Next hop",
                field_owned(cx, "10.0.0.1", false, &next_hop),
            ))
            .child(Self::field_block(
                "Admin distance",
                field_owned(cx, "1", false, &admin_distance),
            ))
            .child(
                div()
                    .text_xs()
                    .text_color(palette::text_faint())
                    .child("Prefix example: 192.0.2.0/24. Admin distance defaults to 1."),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(Self::action_button(
                        "Cancel",
                        true,
                        false,
                        cx,
                        |this, cx| {
                            this.route_edit = None;
                            this.close_modal();
                            cx.notify();
                        },
                    ))
                    .child(Self::action_button(
                        if self.config.preview_loading {
                            "Applying…"
                        } else {
                            "Save route"
                        },
                        !prefix.is_empty() && !next_hop.is_empty() && !self.config.preview_loading,
                        true,
                        cx,
                        ReefApp::edit_route_submit,
                    )),
            );

        self.modal_shell(
            title.into(),
            subtitle.into(),
            body.into_any_element(),
            vec![
                (
                    "Cancel".into(),
                    true,
                    false,
                    Box::new(cx.listener(|this, _, _, cx| {
                        this.route_edit = None;
                        this.close_modal();
                        cx.notify();
                    })),
                ),
                (
                    "Save route".into(),
                    !prefix.is_empty() && !next_hop.is_empty() && !self.config.preview_loading,
                    true,
                    Box::new(cx.listener(
                        |this: &mut ReefApp,
                         _: &MouseDownEvent,
                         _: &mut Window,
                         cx: &mut Context<ReefApp>| {
                            ReefApp::edit_route_submit(this, cx);
                        },
                    )),
                ),
            ],
            cx,
        )
    }

    fn system_settings_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let error = self.modal_error.clone();
        let body = div()
            .flex()
            .flex_col()
            .gap_3()
            .w_full()
            .child(Self::field_block("Hostname", self.draft.hostname.clone()))
            .child(Self::field_block("Domain name", self.draft.domain_name.clone()))
            .child(Self::field_block(
                "Name servers (comma separated)",
                self.draft.name_servers.clone(),
            ))
            .child(
                div()
                    .text_xs()
                    .text_color(palette::text_faint())
                    .child("Set at least one field to generate a preview. Blank fields preserve current values."),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(Self::action_button(
                        "Cancel",
                        true,
                        false,
                        cx,
                        |this, cx| {
                            this.close_modal();
                            cx.notify();
                        },
                    ))
                    .child(Self::action_button(
                        if self.config.preview_loading {
                            "Generating…"
                        } else {
                            "Generate preview"
                        },
                        !self.config.preview_loading,
                        true,
                        cx,
                        ReefApp::preview_configuration,
                    )),
            );

        let mut shell = self.modal_shell(
            "System settings".into(),
            "Preview hostname, domain, and name server changes before applying.".into(),
            body.into_any_element(),
            vec![
                (
                    "Cancel".into(),
                    true,
                    false,
                    Box::new(cx.listener(|this, _, _, cx| {
                        this.close_modal();
                        cx.notify();
                    })),
                ),
                (
                    "Generate preview".into(),
                    !self.config.preview_loading,
                    true,
                    Box::new(cx.listener(
                        |this: &mut ReefApp,
                         _: &MouseDownEvent,
                         _: &mut Window,
                         cx: &mut Context<ReefApp>| {
                            ReefApp::preview_configuration(this, cx);
                        },
                    )),
                ),
            ],
            cx,
        );
        if let Some(error) = error {
            shell = div()
                .flex()
                .flex_col()
                .gap_2()
                .w_full()
                .child(error_panel(&error))
                .child(shell)
                .into_any_element();
        }
        shell
    }

    fn confirm_discard_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        self.modal_shell(
            "Discard unsaved changes?".into(),
            "You have unsaved changes in this dialog. Discard them and close?".into(),
            div()
                .flex()
                .flex_col()
                .gap_3()
                .w_full()
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(Self::action_button(
                            "Keep editing",
                            true,
                            false,
                            cx,
                            |this, cx| {
                                this.resume_modal(cx);
                                cx.notify();
                            },
                        ))
                        .child(Self::action_button(
                            "Discard",
                            true,
                            true,
                            cx,
                            |this, cx| {
                                this.discard_modal(cx);
                                cx.notify();
                            },
                        )),
                )
                .into_any_element(),
            vec![],
            cx,
        )
    }

    fn confirm_apply_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let preview = self.config.preview.as_ref();
        let command_count = preview.map(|p| p.commands.len()).unwrap_or(0);
        self.modal_shell(
            "Apply configuration?".into(),
            format!(
                "This will send {} command(s) to the device. Review the preview on the Configuration page first.",
                command_count
            ).into(),
            div()
                .flex()
                .flex_col()
                .gap_3()
                .w_full()
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(Self::action_button(
                            "Cancel",
                            true,
                            false,
                            cx,
                            |this, cx| {
                                this.close_modal();
                                cx.notify();
                            },
                        ))
                        .child(Self::action_button(
                            "Apply",
                            !self.config.apply_loading,
                            true,
                            cx,
                            ReefApp::apply_configuration,
                        )),
                )
                .into_any_element(),
            vec![],
            cx,
        )
    }

    fn confirm_restore_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let backup_id = self.pending_restore.clone().unwrap_or_default();
        self.modal_shell(
            "Restore configuration?".into(),
            format!(
                "This will replace the running configuration with backup {}. This cannot be undone.",
                backup_id
            ).into(),
            div()
                .flex()
                .flex_col()
                .gap_3()
                .w_full()
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(Self::action_button(
                            "Cancel",
                            true,
                            false,
                            cx,
                            |this, cx| {
                                this.pending_restore = None;
                                this.close_modal();
                                cx.notify();
                            },
                        ))
                        .child(Self::action_button(
                            "Restore",
                            !self.config.apply_loading,
                            true,
                            cx,
                            |this, cx| {
                                if let Some(id) = this.pending_restore.take() {
                                    this.restore_configuration(id, cx);
                                }
                                this.close_modal();
                                cx.notify();
                            },
                        )),
                )
                .into_any_element(),
            vec![],
            cx,
        )
    }

    fn modal_shell(
        &mut self,
        title: SharedString,
        subtitle: SharedString,
        body: AnyElement,
        actions: Vec<(
            SharedString,
            bool,
            bool,
            Box<dyn Fn(&MouseDownEvent, &mut Window, &mut App)>,
        )>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut actions_row = div().flex().gap_2().w_full();
        for (label, enabled, _primary, on_click) in actions {
            let mut button = div()
                .flex()
                .items_center()
                .justify_center()
                .h(px(34.))
                .px_3()
                .rounded_md()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child(label);
            if enabled {
                button = button
                    .bg(palette::accent())
                    .text_color(palette::background())
                    .cursor_pointer()
                    .hover(|style| style.opacity(0.85))
                    .on_mouse_down(MouseButton::Left, on_click);
            } else {
                button = button.opacity(0.45);
            }
            actions_row = actions_row.child(button);
        }

        div()
            .flex()
            .flex_col()
            .gap_3()
            .w_full()
            .max_w(px(520.))
            .p_5()
            .rounded_lg()
            .border_1()
            .border_color(palette::panel_border())
            .bg(palette::panel())
            .shadow_lg()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_base()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(palette::text())
                            .child(title),
                    )
                    .child(
                        div()
                            .text_sm()
                            .line_height(px(19.))
                            .text_color(palette::text_muted())
                            .child(subtitle),
                    ),
            )
            .child(div().h(px(1.)).bg(palette::panel_border()))
            .child(body)
            .child(actions_row)
            .into_any_element()
    }
}

impl ConnectionForm {
    /// Plain-data copy used by the app state. The text field entities stay
    /// alive because `ReefApp` keeps their handles as well.
    fn clone_for_state(&self) -> Self {
        Self {
            profile: self.profile.clone(),
            password: self.password.clone(),
            enable_secret: self.enable_secret.clone(),
            trust_host_key: self.trust_host_key,
            connecting: self.connecting,
            error: self.error.clone(),
        }
    }
}

impl Connected {
    fn clone_for_state(&self) -> Self {
        Self {
            profile: self.profile.clone(),
            identity: self.identity.clone(),
            identity_error: self.identity_error.clone(),
            privilege_secret: self.privilege_secret.clone(),
            enabling: self.enabling,
        }
    }
}

impl Clone for ConnectionForm {
    fn clone(&self) -> Self {
        self.clone_for_state()
    }
}

fn field<T: 'static>(
    cx: &mut Context<T>,
    placeholder: &'static str,
    masked: bool,
    initial: &'static str,
) -> Entity<TextField> {
    let field = cx.new(|cx| TextField::new(cx, placeholder, masked));
    if !initial.is_empty() {
        field.update(cx, |field, cx| field.set_content(initial, cx));
    }
    field
}

fn field_owned(
    cx: &mut Context<ReefApp>,
    placeholder: &'static str,
    masked: bool,
    initial: &str,
) -> Entity<TextField> {
    let field = cx.new(|cx| TextField::new(cx, placeholder, masked));
    if !initial.is_empty() {
        field.update(cx, |field, cx| field.set_content(initial, cx));
    }
    field
}

fn set_submit(
    weak: WeakEntity<ReefApp>,
    field: &Entity<TextField>,
    cx: &mut Context<ReefApp>,
    action: fn(&mut ReefApp, &mut Context<ReefApp>),
) {
    field.update(cx, |field, _| {
        let weak = weak.clone();
        field.set_on_submit(move |_text, _window, cx| {
            let weak = weak.clone();
            cx.defer(move |cx| {
                weak.update(cx, action).ok();
            });
        });
    });
}

fn set_submit_with_text(
    weak: WeakEntity<ReefApp>,
    field: &Entity<TextField>,
    cx: &mut Context<ReefApp>,
    action: fn(&mut ReefApp, String, &mut Context<ReefApp>),
) {
    field.update(cx, |field, _| {
        let weak = weak.clone();
        field.set_on_submit(move |text, _window, cx| {
            let weak = weak.clone();
            cx.defer(move |cx| {
                weak.update(cx, |this, cx| action(this, text, cx)).ok();
            });
        });
    });
}

fn non_empty(value: String) -> Option<String> {
    let value = value.trim().to_string();
    (!value.is_empty()).then_some(value)
}

trait SessionTransportExt {
    fn into_session(self) -> SessionTransport;
}

impl SessionTransportExt for ProfileTransport {
    fn into_session(self) -> SessionTransport {
        match self {
            ProfileTransport::Ssh => SessionTransport::Ssh,
            ProfileTransport::Telnet => SessionTransport::Telnet,
            ProfileTransport::Serial => SessionTransport::Serial,
        }
    }
}

fn transport_label(transport: &ProfileTransport) -> String {
    match transport {
        ProfileTransport::Ssh => "SSH".to_string(),
        ProfileTransport::Telnet => "Telnet".to_string(),
        ProfileTransport::Serial => "Serial".to_string(),
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn format_uptime(seconds: u64) -> String {
    format!("{}m {}s", seconds / 60, seconds % 60)
}

fn join_or_none(values: &[String]) -> String {
    if values.is_empty() {
        "None".to_string()
    } else {
        values.join(", ")
    }
}

fn normalize_terminal(output: &str) -> String {
    output.replace("\r\n", "\n").replace('\r', "\n")
}

// ----------------------------------------------------------------------
// Small presentational helpers
// ----------------------------------------------------------------------

fn heading(value: impl Into<SharedString>) -> AnyElement {
    div()
        .text_lg()
        .font_weight(FontWeight::BOLD)
        .text_color(palette::text())
        .child(value.into())
        .into_any_element()
}

fn paragraph(value: impl Into<SharedString>) -> AnyElement {
    div()
        .text_sm()
        .line_height(px(20.))
        .text_color(palette::text_muted())
        .child(value.into())
        .into_any_element()
}

fn section_label(value: impl Into<SharedString>) -> AnyElement {
    div()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(palette::text_faint())
        .child(value.into())
        .into_any_element()
}

fn panel(
    title: impl Into<SharedString>,
    subtitle: impl Into<SharedString>,
    body: AnyElement,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .w_full()
        .p_4()
        .rounded_lg()
        .border_1()
        .border_color(palette::panel_border())
        .bg(palette::panel())
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title.into()),
                )
                .child(paragraph(subtitle)),
        )
        .child(body)
        .into_any_element()
}

fn error_panel(message: &str) -> AnyElement {
    div()
        .w_full()
        .p_3()
        .rounded_md()
        .border_1()
        .border_color(palette::danger())
        .bg(palette::panel_alt())
        .text_sm()
        .text_color(palette::danger())
        .child(message.to_string())
        .into_any_element()
}

fn error_panel_with_body(title: &str, body: AnyElement) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .w_full()
        .p_3()
        .rounded_md()
        .border_1()
        .border_color(palette::danger())
        .bg(palette::panel_alt())
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(palette::danger())
                .child(title.to_string()),
        )
        .child(body)
        .into_any_element()
}

fn notice_panel(message: impl Into<SharedString>) -> AnyElement {
    div()
        .w_full()
        .p_3()
        .rounded_md()
        .border_1()
        .border_color(palette::accent_dim())
        .bg(palette::panel_alt())
        .text_sm()
        .text_color(palette::accent_strong())
        .child(message.into())
        .into_any_element()
}

fn safety_banner() -> AnyElement {
    let message = if STARTUP_CONFIG_WRITES_ENABLED {
        "Startup-configuration writes are enabled."
    } else {
        "Startup-configuration writes are permanently disabled."
    };

    div()
        .flex()
        .flex_col()
        .gap_1()
        .w_full()
        .p_4()
        .rounded_lg()
        .border_1()
        .border_color(palette::accent_dim())
        .bg(palette::panel())
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(palette::accent_strong())
                .child(message),
        )
        .child(paragraph(
            "Reef blocks write memory, write, copy running-config startup-config, copy run start, and equivalent forms in the Rust backend. Running-config edits and show commands still work. This restriction cannot be re-enabled in the UI.",
        ))
        .child(
            div()
                .text_xs()
                .text_color(palette::text_faint())
                .child("Limitation: arbitrary terminal access cannot be fully guarded. Devices that ignore the blocked command list or out-of-band management tools are outside this control."),
        )
        .into_any_element()
}

fn stat_pill(value: &str) -> AnyElement {
    div()
        .px_2()
        .py_1()
        .rounded_full()
        .bg(palette::panel_alt())
        .border_1()
        .border_color(palette::panel_border())
        .text_xs()
        .text_color(palette::text_muted())
        .child(value.to_string())
        .into_any_element()
}

fn stat_card(label: &str, value: String) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .flex_1()
        .min_w(px(160.))
        .p_3()
        .rounded_md()
        .border_1()
        .border_color(palette::panel_border())
        .bg(palette::panel_alt())
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(palette::text_faint())
                .child(label.to_string()),
        )
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(palette::text())
                .child(value),
        )
        .into_any_element()
}

fn info_cell(label: &str, value: String) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .flex_1()
        .min_w(px(220.))
        .child(
            div()
                .text_xs()
                .text_color(palette::text_faint())
                .child(label.to_string()),
        )
        .child(div().text_sm().text_color(palette::text()).child(value))
        .into_any_element()
}

fn capability_grid(identity: &DeviceIdentity) -> AnyElement {
    let capabilities = [
        (
            "Layer 2 switching",
            identity.capabilities.supports_l2_switchports,
        ),
        ("IPv6", identity.capabilities.supports_ipv6),
        (
            "Static routing",
            identity.capabilities.supports_static_routing,
        ),
        (
            "Dynamic routing",
            identity.capabilities.supports_dynamic_routing,
        ),
        ("NAT", identity.capabilities.supports_nat),
        ("DHCP", identity.capabilities.supports_dhcp),
        ("ACLs", identity.capabilities.supports_acl),
    ];

    let mut grid = div().flex().flex_wrap().gap_2().w_full();
    for (label, enabled) in capabilities {
        grid = grid.child(stat_pill(&format!(
            "{} · {}",
            if enabled { "Supported" } else { "Not detected" },
            label
        )));
    }
    grid.into_any_element()
}

fn quick_card(label: &'static str, section: Section, cx: &mut Context<ReefApp>) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_4()
        .flex_1()
        .min_w(px(180.))
        .p_4()
        .rounded_lg()
        .border_1()
        .border_color(palette::panel_border())
        .bg(palette::panel())
        .cursor_pointer()
        .hover(|style| style.bg(palette::panel_alt()))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(div().text_color(palette::accent()).child("◇"))
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(label),
                ),
        )
        .child(div().text_color(palette::text_faint()).child("↗"))
        .id(("quick-card", section.index()))
        .on_click(cx.listener(move |this, _, _, cx| this.navigate(section, cx)))
        .into_any_element()
}

fn table_header(columns: &[&str]) -> AnyElement {
    let mut row = div()
        .flex()
        .flex_row()
        .gap_2()
        .w_full()
        .px_2()
        .py_1()
        .rounded_md()
        .bg(palette::panel_alt());
    for column in columns {
        row = row.child(
            div()
                .flex_1()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(palette::text_faint())
                .child(column.to_string()),
        );
    }
    row.into_any_element()
}

fn table_row(cells: Vec<String>, up: bool) -> AnyElement {
    let mut row = div()
        .flex()
        .flex_row()
        .gap_2()
        .w_full()
        .px_2()
        .py_1()
        .border_b_1()
        .border_color(palette::panel_border());
    for cell in cells {
        row = row.child(
            div()
                .flex_1()
                .text_sm()
                .text_color(if up {
                    palette::accent_strong()
                } else {
                    palette::text()
                })
                .child(cell),
        );
    }
    row.into_any_element()
}

fn config_list_row(cells: Vec<String>) -> AnyElement {
    let mut row = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_2()
        .w_full()
        .px_2()
        .py_1()
        .border_b_1()
        .border_color(palette::panel_border());
    for (index, cell) in cells.into_iter().enumerate() {
        let mut cell_div = div()
            .text_sm()
            .text_color(if index == 0 {
                palette::text()
            } else {
                palette::text_muted()
            })
            .child(cell);
        if index == 0 {
            cell_div = cell_div.font_weight(FontWeight::SEMIBOLD);
        }
        row = row.child(cell_div);
    }
    row.into_any_element()
}

fn code_line(value: String) -> AnyElement {
    div()
        .w_full()
        .px_2()
        .py_1()
        .font_family(".SystemMonospaceFont")
        .text_xs()
        .text_color(palette::text_muted())
        .child(value)
        .into_any_element()
}

fn code_block(id: &'static str, value: String) -> AnyElement {
    div()
        .id(id)
        .w_full()
        .max_h(px(320.))
        .overflow_y_scroll()
        .p_3()
        .rounded_md()
        .border_1()
        .border_color(palette::panel_border())
        .bg(palette::terminal_bg())
        .font_family(".SystemMonospaceFont")
        .text_xs()
        .line_height(px(18.))
        .text_color(palette::text())
        .whitespace_normal()
        .child(value)
        .into_any_element()
}

fn checkbox_row(label: &'static str, checked: bool, cx: &mut Context<ReefApp>) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .child(
            div()
                .size(px(16.))
                .rounded_md()
                .border_1()
                .border_color(if checked {
                    palette::accent()
                } else {
                    palette::input_border()
                })
                .bg(if checked {
                    palette::accent_dim()
                } else {
                    palette::input_bg()
                })
                .flex()
                .items_center()
                .justify_center()
                .text_xs()
                .text_color(palette::accent_strong())
                .child(if checked { "✓" } else { "" }),
        )
        .child(
            div()
                .text_sm()
                .text_color(palette::text_muted())
                .child(label),
        )
        .id("trust-host-key")
        .on_click(cx.listener(|this, _, _, cx| {
            if let Some(form) = this.connection.as_mut() {
                form.trust_host_key = !form.trust_host_key;
            }
            cx.notify();
        }))
        .into_any_element()
}

fn empty_connected_panel(
    title: &'static str,
    message: &'static str,
    cx: &mut Context<ReefApp>,
) -> AnyElement {
    panel(
        title,
        message,
        div()
            .flex()
            .justify_start()
            .child(ReefApp::action_button(
                "Open connections",
                true,
                true,
                cx,
                |this, cx| this.navigate(Section::Connections, cx),
            ))
            .into_any_element(),
    )
}

fn parser_warnings(warnings: &[String]) -> AnyElement {
    let mut list = div().flex().flex_col().gap_1().w_full();
    for warning in warnings {
        list = list.child(paragraph(warning.clone()));
    }
    panel(
        "Parser warnings",
        &format!(
            "{} lines were not fully understood; raw configuration remains on the device",
            warnings.len()
        ),
        list.into_any_element(),
    )
}
