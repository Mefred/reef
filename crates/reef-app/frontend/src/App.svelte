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
