use serde_json::{Value, json};

const VERSION: &str = "0.9.3";
const PROTOCOL: u32 = 22;
const ENDPOINT_GENERATION: u32 = 1;

/// What `herdr status server --json` and `herdr status client --json` report.
///
/// It starts out as a Herdr 0.9.3 whose server runs the same release as the executable. The
/// compatibility verdicts are derived from the numbers as Herdr derives them: a server is
/// compatible when its protocol and endpoint generation equal the executable's, and a server that
/// reports no endpoint generation has no verdict.
#[derive(Clone, Debug)]
pub struct HerdrStatus {
    executable: Release,
    server: Option<Release>,
}

#[derive(Clone, Debug)]
struct Release {
    version: String,
    protocol: u32,
    endpoint_generation: Option<u32>,
}

impl Release {
    fn herdr_0_9_3() -> Self {
        Self {
            version: VERSION.to_owned(),
            protocol: PROTOCOL,
            endpoint_generation: Some(ENDPOINT_GENERATION),
        }
    }
}

impl HerdrStatus {
    /// An executable and a running server of the same release.
    pub fn compatible() -> Self {
        Self {
            executable: Release::herdr_0_9_3(),
            server: Some(Release::herdr_0_9_3()),
        }
    }

    /// An executable and a session whose server is not running.
    pub fn server_not_running() -> Self {
        Self {
            server: None,
            ..Self::compatible()
        }
    }

    /// Makes the running server report `version`.
    pub fn server_version(mut self, version: &str) -> Self {
        self.running_server().version = version.to_owned();
        self
    }

    /// Makes the running server speak private protocol `protocol`.
    pub fn server_protocol(mut self, protocol: u32) -> Self {
        self.running_server().protocol = protocol;
        self
    }

    /// Makes the running server report endpoint protocol generation `generation`, or none.
    pub fn server_endpoint_generation(mut self, generation: Option<u32>) -> Self {
        self.running_server().endpoint_generation = generation;
        self
    }

    fn running_server(&mut self) -> &mut Release {
        self.server
            .as_mut()
            .expect("the scripted server is not running")
    }

    pub(crate) fn server_json(&self) -> Value {
        let Some(server) = &self.server else {
            return json!({
                "status": "not_running",
                "running": false,
                "version": null,
                "protocol": null,
                "capabilities": null,
                "compatible": null,
                "endpoint_compatible": null,
                "restart_needed": false,
                "server_binary_stale": false,
            });
        };
        let endpoint_compatible = server
            .endpoint_generation
            .map(|generation| Some(generation) == self.executable.endpoint_generation);
        json!({
            "status": "running",
            "running": true,
            "version": server.version,
            "protocol": server.protocol,
            "capabilities": {
                "live_handoff": true,
                "detached_server_daemon": true,
                "endpoint_protocol_generation": server.endpoint_generation,
                "surface_interest": true,
                "health_check": true,
                "ssh_agent_registration": true,
            },
            "compatible": server.protocol == self.executable.protocol,
            "endpoint_compatible": endpoint_compatible,
            "restart_needed": endpoint_compatible != Some(true),
            "server_binary_stale": server.version != self.executable.version,
        })
    }

    pub(crate) fn client_json(&self) -> Value {
        json!({
            "version": self.executable.version,
            "channel": "stable",
            "protocol": self.executable.protocol,
            "endpoint_protocol_generation": self.executable.endpoint_generation,
            "endpoint_capabilities": ["surface_interest", "presentation_effects_fence", "health_check"],
            "remote_host_bridge": true,
            "remote_bridge_idle_timeout": true,
        })
    }
}
