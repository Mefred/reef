<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import TerminalPanel from "./lib/TerminalPanel.svelte";

  type Transport = "Ssh" | "Telnet" | "Serial";
  interface DeviceIdentity {
    hostname: string;
    iosxe_version: string;
    platform: string;
    chassis_serial: string;
    processor: string;
    interfaces: Array<{ name: string; oper_status: string | null }>;
    capabilities: {
      supports_l2_switchports: boolean;
      supports_ipv6: boolean;
      supports_static_routing: boolean;
      supports_dynamic_routing: boolean;
      supports_nat: boolean;
      supports_dhcp: boolean;
      supports_acl: boolean;
    };
  }

  interface DeviceWorkspace {
    interfaces: Array<{ name: string; oper_status: string | null }>;
    interfaceError: string | null;
    vlans: Array<{ id: number; name: string; enabled: boolean }>;
    vlanError: string | null;
    staticRoutes: Array<{ prefix: string; next_hop: string; admin_distance: number | null; permanent: boolean }>;
    routeError: string | null;
  }

  interface ConfigurationSnapshot {
    hostname: string | null;
    domainName: string | null;
    nameServers: string[];
    interfaces: Array<{ name: string; description: string | null; enabled: boolean; mode: string; ipv4: string | null; mtu: number | null }>;
    vlans: Array<{ id: number; name: string; enabled: boolean }>;
    staticRoutes: string[];
    acls: Array<{ name: string; aclType: string; rules: string[] }>;
    dhcpPools: Array<{ name: string; network: string; defaultRouter: string | null; dnsServers: string[] }>;
    warnings: string[];
  }

  interface DeviceProfile {
    id: string;
    name: string;
    host: string;
    port: number;
    transport: Transport;
    username: string;
  }

  let activeSection = $state("Overview");
  let profiles = $state<DeviceProfile[]>([]);
  let loadingProfiles = $state(false);
  let profileError = $state<string | null>(null);
  let profileName = $state("");
  let profileHost = $state("");
  let profilePort = $state(22);
  let profileTransport = $state<Transport>("Ssh");
  let profileUsername = $state("admin");
  let savingProfile = $state(false);
  let profileFormError = $state<string | null>(null);
  let selectedProfile = $state<DeviceProfile | null>(null);
  let password = $state("");
  let enableSecret = $state("");
  let privilegeSecret = $state("");
  let enablingDevice = $state(false);
  let connecting = $state(false);
  let connectionError = $state<string | null>(null);
  let trustUnknownHostKey = $state(false);
  let connectedProfile = $state<DeviceProfile | null>(null);
  let deviceIdentity = $state<DeviceIdentity | null>(null);
  let identityError = $state<string | null>(null);
  let disconnecting = $state(false);
  let workspace = $state<DeviceWorkspace | null>(null);
  let workspaceLoading = $state(false);
  let workspaceRefreshError = $state<string | null>(null);
  let configuration = $state<ConfigurationSnapshot | null>(null);
  let configurationLoading = $state(false);
  let configurationError = $state<string | null>(null);

  async function refreshConfiguration(deviceId: string) {
    configurationLoading = true;
    configurationError = null;
    try {
      configuration = await invoke<ConfigurationSnapshot>("get_device_configuration", { deviceId });
    } catch (error) {
      configurationError = String(error);
    } finally {
      configurationLoading = false;
    }
  }

  async function refreshWorkspace(deviceId: string) {
    workspaceLoading = true;
    workspaceRefreshError = null;
    try {
      workspace = await invoke<DeviceWorkspace>("inspect_device_workspace", { deviceId });
    } catch (error) {
      workspaceRefreshError = String(error);
    } finally {
      workspaceLoading = false;
    }
  }

  $effect(() => {
    const deviceId = connectedProfile?.id;
    if (deviceId && ["Interfaces", "VLANs", "Routing"].includes(activeSection)) {
      void refreshWorkspace(deviceId);
    }
    if (deviceId && activeSection === "Configuration") {
      void refreshConfiguration(deviceId);
    }
  });


  const navigation = [
    { label: "Overview", icon: "⌂" },
    { label: "Interfaces", icon: "▤" },
    { label: "VLANs", icon: "◇" },
    { label: "Routing", icon: "⇄" },
    { label: "Configuration", icon: "⌘" },
    { label: "Terminal", icon: ">_" },
  ];

  async function loadProfiles() {
    loadingProfiles = true;
    profileError = null;
    try {
      profiles = await invoke<DeviceProfile[]>("list_profiles");
    } catch (error) {
      profileError = String(error);
    } finally {
      loadingProfiles = false;
    }
  }

  async function saveProfile() {
    profileFormError = null;
    savingProfile = true;
    try {
      const profile = await invoke<DeviceProfile>("save_profile", {
        request: {
          name: profileName,
          host: profileHost,
          port: profileTransport === "Telnet" ? 23 : profilePort,
          transport: profileTransport,
          username: profileUsername,
        },
      });
      profiles = [...profiles, profile].sort((a, b) => a.name.localeCompare(b.name));
      profileName = "";
      profileHost = "";
      profilePort = profileTransport === "Telnet" ? 23 : 22;
      profileFormError = null;
    } catch (error) {
      profileFormError = String(error);
    } finally {
      savingProfile = false;
    }
  }

  async function deleteProfile(profile: DeviceProfile) {
    try {
      await invoke("delete_profile", { profileId: profile.id });
      profiles = profiles.filter((item) => item.id !== profile.id);
      if (selectedProfile?.id === profile.id) selectedProfile = null;
    } catch (error) {
      profileError = String(error);
    }
  }

  async function connectProfile(profile: DeviceProfile) {
    connecting = true;
    connectionError = null;
    try {
      if (connectedProfile && connectedProfile.id !== profile.id) {
        await invoke("disconnect_device", { deviceId: connectedProfile.id });
        connectedProfile = null;
        deviceIdentity = null;
        workspace = null;
        workspaceRefreshError = null;
        configuration = null;
        configurationError = null;
        identityError = null;
      }
      await invoke("connect_device", {
        request: {
          deviceId: profile.id,
          host: profile.host,
          port: profile.port,
          transport: profile.transport,
          username: profile.username,
          password: password || null,
          enableSecret: enableSecret || null,
          serialPort: profile.transport === "Serial" ? profile.host : null,
          serialBaud: 9600,
          acceptUnknownHostKey: trustUnknownHostKey,
        },
      });
      connectedProfile = profile;
      workspace = null;
      workspaceRefreshError = null;
      configuration = null;
      configurationError = null;
      selectedProfile = null;
      password = "";
      activeSection = "Overview";
      deviceIdentity = null;
      identityError = null;
      privilegeSecret = "";
      const suppliedEnableSecret = enableSecret;
      enableSecret = "";
      if (suppliedEnableSecret) {
        try {
          await invoke("enable_device", {
            deviceId: profile.id,
            secret: suppliedEnableSecret,
          });
        } catch (error) {
          identityError = String(error);
        }
      }
      if (!identityError) {
        try {
          deviceIdentity = await invoke<DeviceIdentity>("identify_device", { deviceId: profile.id });
        } catch (error) {
          identityError = String(error);
        }
      }
    } catch (error) {
      connectionError = String(error);
    } finally {
      connecting = false;
    }
  }

  async function enableAndIdentify() {
    if (!connectedProfile) return;
    enablingDevice = true;
    identityError = null;
    try {
      await invoke("enable_device", {
        deviceId: connectedProfile.id,
        secret: privilegeSecret,
      });
      deviceIdentity = await invoke<DeviceIdentity>("identify_device", {
        deviceId: connectedProfile.id,
      });
      privilegeSecret = "";
    } catch (error) {
      identityError = String(error);
    } finally {
      enablingDevice = false;
    }
  }

  async function disconnectDevice() {
    if (!connectedProfile) return;
    disconnecting = true;
    try {
      await invoke("disconnect_device", { deviceId: connectedProfile.id });
      connectedProfile = null;
      deviceIdentity = null;
      workspace = null;
      workspaceRefreshError = null;
      configuration = null;
      configurationError = null;
      identityError = null;
    } catch (error) {
      connectionError = String(error);
    } finally {
      disconnecting = false;
    }
  }

  onMount(() => {
    void loadProfiles();
  });
</script>

<svelte:head>
  <title>Reef — Network Management</title>
</svelte:head>

<div class="app-shell">
  <aside class="sidebar">
    <a class="brand" href="/" aria-label="Reef home">
      <span class="brand-mark">r</span>
      <span>reef<span class="brand-period">.</span></span>
    </a>

    <div class="workspace-label">WORKSPACE</div>
    <nav aria-label="Main navigation">
      {#each navigation as item}
        <button
          class:active={activeSection === item.label}
          class="nav-item"
          onclick={() => (activeSection = item.label)}
        >
          <span class="nav-icon">{item.icon}</span>
          <span>{item.label}</span>
        </button>
      {/each}
      <button
        class:active={activeSection === "Connections"}
        class="nav-item"
        onclick={() => (activeSection = "Connections")}
      >
        <span class="nav-icon">⌁</span>
        <span>Connections</span>
      </button>
    </nav>

    <div class="sidebar-footer">
      <span class:connected={connectedProfile} class="status-dot"></span>
      <span>{connectedProfile ? connectedProfile.name : "Not connected"}</span>
    </div>
  </aside>

  <main class="main-area">
    <header class="topbar">
      <div class="breadcrumbs">Workspace <span>/</span> {activeSection}</div>
      <div class="topbar-actions">
        {#if connectedProfile}
          <span class="connection-label"><span class="status-dot connected"></span> {connectedProfile.name}</span>
          <button class="secondary-button compact" onclick={() => void disconnectDevice()} disabled={disconnecting}>
            {disconnecting ? "Disconnecting…" : "Disconnect"}
          </button>
        {:else}
          <span class="connection-label"><span class="status-dot muted"></span> No device connected</span>
        {/if}
      </div>
    </header>

    <section class="content">
      <div class="page-heading">
        <div>
          <p class="eyebrow">NETWORK WORKSPACE</p>
          <h1>{activeSection}</h1>
          <p class="subtitle">Manage your network devices from one place.</p>
        </div>
        <button class="primary-button" onclick={() => (activeSection = "Connections")}>
          <span>＋</span> Connect device
        </button>
      </div>

      {#if activeSection === "Overview"}
        {#if connectedProfile}
          <div class="welcome-panel connected-panel">
            <div class="welcome-copy">
              <div class="welcome-icon">✓</div>
              <h2>Connected to {connectedProfile.name}</h2>
              {#if deviceIdentity}
                <p>{deviceIdentity.hostname} · {deviceIdentity.platform} · IOS XE {deviceIdentity.iosxe_version}</p>
                <div class="device-summary">
                  <span><strong>{deviceIdentity.interfaces.length}</strong> discovered interfaces</span>
                  <span><strong>{deviceIdentity.processor || "Unknown"}</strong> processor</span>
                </div>
                <span class="coming-soon">DEVICE IDENTIFIED · CONFIGURATION VIEWS IN PROGRESS</span>
              {:else if identityError}
                <p class="form-error">Connected, but device identification failed: {identityError}</p>
                <div class="privilege-panel">
                    <label>Enable secret
                      <input type="password" bind:value={privilegeSecret} autocomplete="off" placeholder="Privilege escalation secret" />
                    </label>
                    <button class="primary-button" onclick={() => void enableAndIdentify()} disabled={enablingDevice}>
                      {enablingDevice ? "Enabling…" : "Enable and identify"}
                    </button>
                </div>
                <span class="coming-soon">SESSION CONNECTED</span>
              {:else}
                <p>{connectedProfile.host}:{connectedProfile.port} · {connectedProfile.transport} · {connectedProfile.username}</p>
                <span class="coming-soon">SESSION ESTABLISHED · IDENTIFYING DEVICE</span>
              {/if}
            </div>
            <div class="abstract-network" aria-hidden="true">
              <div class="network-ring ring-one"></div>
              <div class="network-ring ring-two"></div>
              <div class="network-core">R</div>
              <span class="network-node node-one"></span>
              <span class="network-node node-two"></span>
              <span class="network-node node-three"></span>
            </div>
          </div>
        {:else}
          <div class="welcome-panel">
            <div class="welcome-copy">
              <div class="welcome-icon">⌁</div>
              <h2>Your network workspace</h2>
              <p>Connect to a device to inspect its interfaces, manage configuration, and monitor its status.</p>
              <button class="primary-button" onclick={() => (activeSection = "Connections")}>
                Connect your first device <span>→</span>
              </button>
            </div>
            <div class="abstract-network" aria-hidden="true">
              <div class="network-ring ring-one"></div>
              <div class="network-ring ring-two"></div>
              <div class="network-core">R</div>
              <span class="network-node node-one"></span>
              <span class="network-node node-two"></span>
              <span class="network-node node-three"></span>
            </div>
          </div>
        {/if}

        <div class="section-heading">
          <h2>Quick access</h2>
          <span>{connectedProfile ? "Device workspace views are being migrated" : "Available after connecting"}</span>
        </div>
        <div class="quick-grid">
          {#each ["Interfaces", "VLANs", "Routing", "Configuration"] as item}
            <button class="quick-card" onclick={() => (activeSection = item)}>
              <span class="quick-card-icon">◇</span>
              <span>{item}</span>
              <span class="quick-arrow">↗</span>
            </button>
          {/each}
        </div>
      {:else if activeSection === "Connections"}
        <div class="connection-layout">
          <div class="connection-panel">
            <div class="panel-heading">
              <div>
                <h2>Saved profiles</h2>
                <p>Connection details are stored locally. Passwords are never saved in profiles.</p>
              </div>
              <button class="secondary-button" onclick={() => void loadProfiles()} disabled={loadingProfiles}>
                {loadingProfiles ? "Refreshing…" : "Refresh"}
              </button>
            </div>

            {#if loadingProfiles && profiles.length === 0}
              <div class="profile-state">Loading saved profiles…</div>
            {:else if profileError}
              <div class="profile-state error-state">
                <strong>Unable to load profiles</strong>
                <span>{profileError}</span>
              </div>
            {:else if profiles.length === 0}
              <div class="profile-state">
                <span class="empty-profile-icon">⌁</span>
                <strong>No saved profiles yet</strong>
                <span>Create a profile to save a device's address and connection preferences.</span>
              </div>
            {:else}
              <div class="profile-list">
                {#each profiles as profile (profile.id)}
                  <div class="profile-row">
                    <span class="profile-device-icon">◇</span>
                    <div class="profile-details">
                      <strong>{profile.name}</strong>
                      <span>{profile.host}:{profile.port} · {profile.transport} · {profile.username || "No username"}</span>
                    </div>
                    <button class="profile-action" onclick={() => { selectedProfile = profile; password = ""; enableSecret = ""; trustUnknownHostKey = false; connectionError = null; }}>Connect</button>
                    <button class="delete-action" aria-label="Delete profile" onclick={() => void deleteProfile(profile)}>×</button>
                  </div>
                {/each}
              </div>
            {/if}

            {#if selectedProfile}
              <div class="auth-panel">
                <div class="panel-heading">
                  <div>
                    <h2>Connect to {selectedProfile.name}</h2>
                    <p>{selectedProfile.host} · {selectedProfile.transport}</p>
                  </div>
                  <button class="icon-button" aria-label="Close connection form" onclick={() => (selectedProfile = null)}>×</button>
                </div>
                {#if selectedProfile.transport !== "Serial"}
                  <label>Password
                    <input type="password" bind:value={password} autocomplete="current-password" placeholder="Device login password" />
                  </label>
                  <label>Enable secret <span class="optional-label">Optional</span>
                    <input type="password" bind:value={enableSecret} autocomplete="off" placeholder="Privilege escalation secret" />
                  </label>
                {/if}
                {#if connectionError}
                  <p class="form-error">{connectionError}</p>
                {/if}
                {#if selectedProfile.transport === "Ssh" && connectionError?.toLowerCase().includes("unknown host key")}
                  <label class="trust-key-option">
                    <input type="checkbox" bind:checked={trustUnknownHostKey} />
                    Trust this SSH host key and save it for future connections
                  </label>
                {/if}
                <div class="form-actions">
                  <button class="secondary-button" onclick={() => (selectedProfile = null)}>Cancel</button>
                  <button class="primary-button" onclick={() => void connectProfile(selectedProfile!)} disabled={connecting}>
                    {connecting ? "Connecting…" : "Connect"}
                  </button>
                </div>
                <p class="security-note">Credentials are sent to the Rust session service for this connection only. They are not written to the profile.</p>
              </div>
            {/if}
          </div>

          <div class="connection-note">
            <div class="welcome-icon">＋</div>
            <h2>New connection profile</h2>
            <p>Save a reusable device profile. Authentication secrets are requested separately when connecting.</p>
            <form class="profile-form" onsubmit={(event) => { event.preventDefault(); void saveProfile(); }}>
              <label>Profile name
                <input bind:value={profileName} required maxlength="80" placeholder="Core switch" />
              </label>
              <label>Transport
                <select bind:value={profileTransport}>
                  <option value="Ssh">SSH</option>
                  <option value="Telnet">Telnet</option>
                  <option value="Serial">Serial</option>
                </select>
              </label>
              <label>{profileTransport === "Serial" ? "Serial port" : "Host or IP address"}
                <input bind:value={profileHost} required placeholder={profileTransport === "Serial" ? "/dev/ttyUSB0 or COM3" : "192.168.1.1"} />
              </label>
              {#if profileTransport !== "Serial"}
                <label>Port
                  <input type="number" bind:value={profilePort} min="1" max="65535" required />
                </label>
                <label>Username
                  <input bind:value={profileUsername} required placeholder="admin" />
                </label>
              {/if}
              {#if profileFormError}
                <p class="form-error">{profileFormError}</p>
              {/if}
              <button class="primary-button form-submit" type="submit" disabled={savingProfile}>
                {savingProfile ? "Saving…" : "Save profile"}
              </button>
            </form>
          </div>
        </div>
      {:else if activeSection === "Terminal"}
        {#if connectedProfile}
          <div class="terminal-workspace">
            <div class="terminal-heading">
              <div>
                <h2>{connectedProfile.name}</h2>
                <p>{connectedProfile.host} · {connectedProfile.transport} · command session</p>
              </div>
              <span class="terminal-live"><span class="status-dot connected"></span> Session active</span>
            </div>
            <TerminalPanel deviceId={connectedProfile.id} />
            <p class="terminal-footnote">Commands execute on the connected device. Use device-specific CLI commands; output is returned after each command completes.</p>
          </div>
        {:else}
          <div class="empty-panel">
            <div class="welcome-icon">⌁</div>
            <h2>Connect a device first</h2>
            <p>The terminal becomes available after a device session is established.</p>
            <button class="primary-button" onclick={() => (activeSection = "Connections")}>Open connections</button>
          </div>
        {/if}
      {:else if ["Interfaces", "VLANs", "Routing"].includes(activeSection)}
        <div class="workspace-toolbar">
          <div>
            <h2>{connectedProfile ? connectedProfile.name : "No active device"}</h2>
            <p>Live operational data queried from the connected device.</p>
          </div>
          {#if connectedProfile}
            <button class="secondary-button" onclick={() => void refreshWorkspace(connectedProfile!.id)} disabled={workspaceLoading}>
              {workspaceLoading ? "Refreshing…" : "Refresh data"}
            </button>
          {/if}
        </div>
        {#if !connectedProfile}
          <div class="empty-panel">
            <div class="welcome-icon">⌁</div>
            <h2>Connect a device first</h2>
            <p>Device data is available after establishing a privileged session.</p>
            <button class="primary-button" onclick={() => (activeSection = "Connections")}>Open connections</button>
          </div>
        {:else if workspaceRefreshError}
          <div class="profile-state error-state">
            <strong>Unable to inspect device</strong>
            <span>{workspaceRefreshError}</span>
          </div>
        {:else if workspaceLoading && !workspace}
          <div class="profile-state">Querying device operational state…</div>
        {:else if activeSection === "Interfaces"}
          <div class="data-panel">
            <div class="data-panel-heading">
              <div><h2>Interfaces</h2><p>{workspace?.interfaces.length ?? 0} interfaces returned by IOS CLI</p></div>
              <span class="coming-soon">SHOW IP INTERFACE BRIEF</span>
            </div>
            {#if workspace?.interfaceError}
              <p class="section-error">{workspace.interfaceError}</p>
            {:else if workspace?.interfaces.length}
              <div class="table-scroll"><table class="device-table">
                <thead><tr><th>Interface</th><th>Operational status</th></tr></thead>
                <tbody>{#each workspace.interfaces as iface (iface.name)}
                  <tr><td class="mono">{iface.name}</td><td><span class:state-up={iface.oper_status?.toLowerCase() === "up"} class="state-pill">{iface.oper_status || "Unknown"}</span></td></tr>
                {/each}</tbody>
              </table></div>
            {:else}
              <div class="profile-state">No interface rows were recognized in the device output.</div>
            {/if}
          </div>
        {:else if activeSection === "VLANs"}
          <div class="data-panel">
            <div class="data-panel-heading">
              <div><h2>VLANs</h2><p>{workspace?.vlans.length ?? 0} VLANs returned by IOS CLI</p></div>
              <span class="coming-soon">SHOW VLAN BRIEF</span>
            </div>
            {#if workspace?.vlanError}
              <p class="section-error">{workspace.vlanError}</p>
            {:else if workspace?.vlans.length}
              <div class="table-scroll"><table class="device-table">
                <thead><tr><th>VLAN ID</th><th>Name</th><th>Status</th></tr></thead>
                <tbody>{#each workspace.vlans as vlan (vlan.id)}
                  <tr><td class="mono">{vlan.id}</td><td>{vlan.name}</td><td><span class="state-pill state-up">Active</span></td></tr>
                {/each}</tbody>
              </table></div>
            {:else}
              <div class="profile-state">No VLANs were recognized. This command may not be supported on this device.</div>
            {/if}
          </div>
        {:else}
          <div class="data-panel">
            <div class="data-panel-heading">
              <div><h2>Static routes</h2><p>{workspace?.staticRoutes.length ?? 0} static routes parsed from the routing table</p></div>
              <span class="coming-soon">SHOW IP ROUTE</span>
            </div>
            {#if workspace?.routeError}
              <p class="section-error">{workspace.routeError}</p>
            {:else if workspace?.staticRoutes.length}
              <div class="table-scroll"><table class="device-table">
                <thead><tr><th>Destination</th><th>Next hop</th><th>Administrative distance</th></tr></thead>
                <tbody>{#each workspace.staticRoutes as route, index (route.prefix + '-' + index)}
                  <tr><td class="mono">{route.prefix}</td><td class="mono">{route.next_hop}</td><td>{route.admin_distance ?? "Default"}</td></tr>
                {/each}</tbody>
              </table></div>
            {:else}
              <div class="profile-state">No static routes were recognized. Connected and dynamic routes are not shown in this initial view.</div>
            {/if}
          </div>
        {/if}
      {:else if activeSection === "Configuration"}
        <div class="workspace-toolbar">
          <div><h2>Running configuration</h2><p>Read-only structured snapshot parsed from the connected device. Secrets are excluded from this view.</p></div>
          {#if connectedProfile}
            <button class="secondary-button" onclick={() => void refreshConfiguration(connectedProfile!.id)} disabled={configurationLoading}>
              {configurationLoading ? "Refreshing…" : "Refresh configuration"}
            </button>
          {/if}
        </div>
        {#if !connectedProfile}
          <div class="empty-panel"><div class="welcome-icon">⌁</div><h2>Connect a device first</h2><p>Running configuration is retrieved from an active device session.</p><button class="primary-button" onclick={() => (activeSection = "Connections")}>Open connections</button></div>
        {:else if configurationError}
          <div class="profile-state error-state"><strong>Unable to retrieve configuration</strong><span>{configurationError}</span></div>
        {:else if configurationLoading && !configuration}
          <div class="profile-state">Retrieving and parsing running configuration…</div>
        {:else if configuration}
          <div class="config-summary-grid">
            <article class="config-summary-card"><span>HOSTNAME</span><strong>{configuration.hostname || "Not configured"}</strong></article>
            <article class="config-summary-card"><span>DOMAIN NAME</span><strong>{configuration.domainName || "Not configured"}</strong></article>
            <article class="config-summary-card"><span>INTERFACES</span><strong>{configuration.interfaces.length}</strong></article>
            <article class="config-summary-card"><span>VLANS</span><strong>{configuration.vlans.length}</strong></article>
          </div>
          <div class="data-panel config-section">
            <div class="data-panel-heading"><div><h2>Configured interfaces</h2><p>Configuration state, not live link state</p></div></div>
            {#if configuration.interfaces.length}
              <div class="table-scroll"><table class="device-table"><thead><tr><th>Interface</th><th>Description</th><th>Mode</th><th>IPv4</th><th>Admin state</th></tr></thead>
                <tbody>{#each configuration.interfaces as iface (iface.name)}
                  <tr><td class="mono">{iface.name}</td><td>{iface.description || "—"}</td><td>{iface.mode}</td><td class="mono">{iface.ipv4 || "—"}</td><td><span class:state-up={iface.enabled} class="state-pill">{iface.enabled ? "enabled" : "shutdown"}</span></td></tr>
                {/each}</tbody>
              </table></div>
            {:else}<p class="config-empty">No interface configuration was parsed.</p>{/if}
          </div>
          <div class="config-columns">
            <div class="data-panel config-section">
              <div class="data-panel-heading"><div><h2>VLAN configuration</h2><p>{configuration.vlans.length} configured VLANs</p></div></div>
              {#each configuration.vlans as vlan (vlan.id)}
                <div class="config-list-row"><strong>{vlan.id}</strong><span>{vlan.name || "Unnamed"}</span><span>{vlan.enabled ? "Enabled" : "Shutdown"}</span></div>
              {:else}<p class="config-empty">No VLAN configuration parsed.</p>{/each}
            </div>
            <div class="data-panel config-section">
              <div class="data-panel-heading"><div><h2>Static routes</h2><p>{configuration.staticRoutes.length} configured routes</p></div></div>
              {#each configuration.staticRoutes as route, index (index)}
                <div class="config-list-row"><span class="mono">{route}</span></div>
              {:else}<p class="config-empty">No static routes configured.</p>{/each}
            </div>
          </div>
          <div class="data-panel config-section">
            <div class="data-panel-heading"><div><h2>Other configuration</h2><p>Recognized feature blocks</p></div></div>
            <div class="config-feature-list">
              <span>Name servers: {configuration.nameServers.join(", ") || "None"}</span>
              <span>{configuration.acls.length} ACLs · {configuration.acls.reduce((total, acl) => total + acl.rules.length, 0)} parsed rules</span>
              <span>{configuration.dhcpPools.length} DHCP pools</span>
            </div>
          </div>
          <div class="config-columns">
            <div class="data-panel config-section">
              <div class="data-panel-heading"><div><h2>Access control lists</h2><p>Parsed ACL entries; unsupported syntax is reported below</p></div></div>
              {#each configuration.acls as acl (acl.name)}
                <div class="config-list-row"><strong>{acl.name}</strong><span>{acl.aclType} · {acl.rules.length} rules</span></div>
                {#each acl.rules as rule, index (index)}
                  <div class="config-feature-list"><span class="mono">{rule}</span></div>
                {/each}
              {:else}<p class="config-empty">No ACLs parsed.</p>{/each}
            </div>
            <div class="data-panel config-section">
              <div class="data-panel-heading"><div><h2>DHCP pools</h2><p>{configuration.dhcpPools.length} configured pools</p></div></div>
              {#each configuration.dhcpPools as pool (pool.name)}
                <div class="config-list-row"><strong>{pool.name}</strong><span class="mono">{pool.network}</span></div>
                <div class="config-feature-list">
                  <span>Default router: {pool.defaultRouter || "Not configured"}</span>
                  <span>DNS: {pool.dnsServers.join(", ") || "Not configured"}</span>
                </div>
              {:else}<p class="config-empty">No DHCP pools parsed.</p>{/each}
            </div>
          </div>
          {#if configuration.warnings.length}
            <div class="data-panel config-section">
              <div class="data-panel-heading"><div><h2>Parser warnings</h2><p>{configuration.warnings.length} lines were not fully understood; raw configuration remains on the device</p></div></div>
              <div class="config-warnings">{#each configuration.warnings as warning, index (index)}<p>{warning}</p>{/each}</div>
            </div>
          {/if}
          <p class="security-note">This is a read-only snapshot. Reef does not apply configuration changes from this screen yet. Secret-bearing lines are not returned to the frontend.</p>
        {/if}
      {:else}
        <div class="empty-panel">
          <div class="welcome-icon">◇</div>
          <h2>{activeSection}</h2>
          <p>{connectedProfile ? "This device workspace is being migrated from the previous Slint interface." : "Connect a device to use this workspace."}</p>
          <span class="coming-soon">DEVICE VIEW MIGRATION PENDING</span>
        </div>
      {/if}
    </section>
  </main>
</div>
