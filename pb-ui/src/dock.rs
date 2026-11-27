//! Dock pane ids. The left rail, menus, and layout tree all use this list.

use egui::Color32;
use pb_engine::{EngineId, HostSnapshot};
use serde::{Deserialize, Serialize};

use crate::ext_proto::ExtProto;
use crate::theme::Proto;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DockPane {
    Project,
    Hubs,
    Points,
    /// Removed from the rail; kept so older ui.toml layouts still deserialize.
    #[serde(alias = "Calculations")]
    Calculations,
    Mappings,
    Traffic,
    Streams,
    Modbus,
    Mqtt,
    OpcUa,
    Files,
    /// HTTP / REST JSON clients (Input/Output).
    Http,
    /// LAS well-log files (follow / write). Pack later — no EngineId yet.
    Las,
    /// SNMP agents (Input poll). Pack later — no EngineId yet.
    Snmp,
    /// WITS level 0 record streams.
    Wits,
    /// WebSocket JSON client / server.
    WebSocket,
    /// Sparkplug B host / edge node.
    Sparkplug,
    /// DNP3 master.
    Dnp3,
    /// CAN bus (SocketCAN).
    Can,
    /// Generator (synthetic signals).
    Gen,
    /// One pane that hosts every protocol's config behind a dropdown.
    Protocols,
    /// In-app user guide.
    Help,
}

impl DockPane {
    /// Rail order. Protocol panes are not on the rail: they live inside [`DockPane::Protocols`]
    /// and only enter the layout when popped out.
    pub const ALL: [DockPane; 7] = [
        DockPane::Project,
        DockPane::Hubs,
        DockPane::Points,
        DockPane::Mappings,
        DockPane::Traffic,
        DockPane::Protocols,
        DockPane::Help,
    ];

    /// Every protocol pane, in dropdown order.
    pub const PROTOCOLS: [DockPane; 14] = [
        DockPane::Streams,
        DockPane::Modbus,
        DockPane::Mqtt,
        DockPane::OpcUa,
        DockPane::Files,
        DockPane::Http,
        DockPane::Las,
        DockPane::Snmp,
        DockPane::Wits,
        DockPane::WebSocket,
        DockPane::Sparkplug,
        DockPane::Dnp3,
        DockPane::Can,
        DockPane::Gen,
    ];

    /// True for a protocol pane (one shown inside Protocols or popped out).
    pub fn is_protocol(self) -> bool {
        self.engine().is_some()
    }

    /// True when the service reports this protocol's pack as loaded.
    pub fn pack_loaded(self, host: &HostSnapshot) -> bool {
        self.engine()
            .and_then(|e| e.pack_id())
            .is_some_and(|id| host.packs_loaded.iter().any(|p| p == id))
    }

    /// Protocols the dropdown offers. With no service connected the pack list is unknown, so
    /// every protocol is offered and config can still be edited offline.
    pub fn available_protocols(host: &HostSnapshot) -> Vec<DockPane> {
        if !host.alive {
            return Self::PROTOCOLS.to_vec();
        }
        Self::PROTOCOLS
            .into_iter()
            .filter(|pane| pane.pack_loaded(host))
            .collect()
    }

    /// The ext protocol behind this pane, if any.
    pub fn ext(self) -> Option<ExtProto> {
        match self {
            Self::Wits => Some(ExtProto::Wits),
            Self::WebSocket => Some(ExtProto::WebSocket),
            Self::Sparkplug => Some(ExtProto::Sparkplug),
            Self::Dnp3 => Some(ExtProto::Dnp3),
            Self::Can => Some(ExtProto::Can),
            Self::Gen => Some(ExtProto::Gen),
            _ => None,
        }
    }

    pub fn from_ext(p: ExtProto) -> DockPane {
        match p {
            ExtProto::Wits => Self::Wits,
            ExtProto::WebSocket => Self::WebSocket,
            ExtProto::Sparkplug => Self::Sparkplug,
            ExtProto::Dnp3 => Self::Dnp3,
            ExtProto::Can => Self::Can,
            ExtProto::Gen => Self::Gen,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Project => "Project",
            Self::Hubs => "Hubs",
            Self::Points => "Points",
            Self::Calculations => "Calculations",
            Self::Mappings => "Mappings",
            Self::Traffic => "Traffic",
            Self::Streams => "Streams / TCP",
            Self::Modbus => "Modbus",
            Self::Mqtt => "MQTT",
            Self::OpcUa => "OPC UA",
            Self::Files => "File follow",
            Self::Http => "HTTP / REST",
            Self::Las => "LAS",
            Self::Snmp => "SNMP",
            Self::Wits | Self::WebSocket | Self::Sparkplug | Self::Dnp3 | Self::Can | Self::Gen => self.ext().map(|e| e.title()).unwrap_or(""),
            Self::Protocols => "Protocols",
            Self::Help => "Help",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Hubs => "hubs",
            Self::Points => "points",
            Self::Calculations => "calculations",
            Self::Mappings => "mappings",
            Self::Traffic => "traffic",
            Self::Streams => "streams",
            Self::Modbus => "modbus",
            Self::Mqtt => "mqtt",
            Self::OpcUa => "opcua",
            Self::Files => "files",
            Self::Http => "http",
            Self::Las => "las",
            Self::Snmp => "snmp",
            Self::Wits | Self::WebSocket | Self::Sparkplug | Self::Dnp3 | Self::Can | Self::Gen => self.ext().map(|e| e.id()).unwrap_or(""),
            Self::Protocols => "protocols",
            Self::Help => "help",
        }
    }

    /// Protocol slot, when this pane is a link rather than a bench view.
    pub fn engine(self) -> Option<EngineId> {
        match self {
            Self::Streams => Some(EngineId::Streams),
            Self::Modbus => Some(EngineId::Modbus),
            Self::Mqtt => Some(EngineId::Mqtt),
            Self::OpcUa => Some(EngineId::OpcUa),
            Self::Files => Some(EngineId::FileFollow),
            Self::Http => Some(EngineId::Http),
            Self::Las => Some(EngineId::Las),
            Self::Snmp => Some(EngineId::Snmp),
            Self::Wits | Self::WebSocket | Self::Sparkplug | Self::Dnp3 | Self::Can | Self::Gen => self.ext().map(|e| e.engine()),
            _ => None,
        }
    }

    /// Bright ink for art on the dark scope display.
    pub fn ink_core(self) -> Color32 {
        match self {
            Self::Project => Proto::COPPER,
            Self::Hubs => Proto::COPPER,
            Self::Points => Proto::TEAL,
            Self::Calculations => Proto::INDIGO,
            Self::Mappings => Proto::INDIGO,
            Self::Traffic => Proto::SAGE,
            Self::Streams => Proto::STREAMS,
            Self::Modbus => Proto::MODBUS,
            Self::Mqtt => Proto::MQTT,
            Self::OpcUa => Proto::OPC,
            Self::Files => Proto::FILES,
            Self::Http => Proto::HTTP,
            Self::Las => Proto::LAS,
            Self::Snmp => Proto::SNMP,
            Self::Wits | Self::WebSocket | Self::Sparkplug | Self::Dnp3 | Self::Can | Self::Gen => self.ext().map(|e| e.ink_core()).unwrap_or(Proto::TEAL),
            Self::Protocols => Proto::COPPER,
            Self::Help => Proto::TEAL,
        }
    }

    /// Ink that stays readable on the theme's pane fill.
    pub fn ink(self, light: bool) -> Color32 {
        if !light {
            return self.ink_core();
        }
        match self {
            Self::Project => Proto::COPPER_DEEP,
            Self::Hubs => Proto::COPPER_DEEP,
            Self::Points => Proto::TEAL_DEEP,
            Self::Calculations => Proto::INDIGO_DEEP,
            Self::Mappings => Proto::INDIGO_DEEP,
            Self::Traffic => Proto::SAGE_DEEP,
            Self::Streams => Proto::STREAMS_DEEP,
            Self::Modbus => Proto::MODBUS_DEEP,
            Self::Mqtt => Proto::MQTT_DEEP,
            Self::OpcUa => Proto::OPC_DEEP,
            Self::Files => Proto::FILES_DEEP,
            Self::Http => Proto::HTTP_DEEP,
            Self::Las => Proto::LAS_DEEP,
            Self::Snmp => Proto::SNMP_DEEP,
            Self::Wits | Self::WebSocket | Self::Sparkplug | Self::Dnp3 | Self::Can | Self::Gen => self.ext().map(|e| e.ink_deep()).unwrap_or(Proto::TEAL_DEEP),
            Self::Protocols => Proto::COPPER_DEEP,
            Self::Help => Proto::TEAL_DEEP,
        }
    }
}
