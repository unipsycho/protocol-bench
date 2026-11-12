//! Dock pane ids. The left rail, menus, and layout tree all use this list.

use egui::Color32;
use pb_engine::EngineId;
use serde::{Deserialize, Serialize};

use crate::theme::Proto;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DockPane {
    Project,
    Hubs,
    Points,
    Mappings,
    Traffic,
    Streams,
    Modbus,
    Mqtt,
    OpcUa,
    Files,
}

impl DockPane {
    /// Rail order: bench views, then protocol links.
    pub const ALL: [DockPane; 10] = [
        DockPane::Project,
        DockPane::Hubs,
        DockPane::Points,
        DockPane::Mappings,
        DockPane::Traffic,
        DockPane::Streams,
        DockPane::Modbus,
        DockPane::Mqtt,
        DockPane::OpcUa,
        DockPane::Files,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Project => "Project",
            Self::Hubs => "Hubs",
            Self::Points => "Points",
            Self::Mappings => "Mappings",
            Self::Traffic => "Traffic",
            Self::Streams => "Streams",
            Self::Modbus => "Modbus",
            Self::Mqtt => "MQTT",
            Self::OpcUa => "OPC UA",
            Self::Files => "File follow",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Hubs => "hubs",
            Self::Points => "points",
            Self::Mappings => "mappings",
            Self::Traffic => "traffic",
            Self::Streams => "streams",
            Self::Modbus => "modbus",
            Self::Mqtt => "mqtt",
            Self::OpcUa => "opcua",
            Self::Files => "files",
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
            _ => None,
        }
    }

    pub fn from_engine(id: EngineId) -> Self {
        match id {
            EngineId::Streams => Self::Streams,
            EngineId::Modbus => Self::Modbus,
            EngineId::Mqtt => Self::Mqtt,
            EngineId::OpcUa => Self::OpcUa,
            EngineId::FileFollow => Self::Files,
        }
    }

    /// Bright ink for art on the dark scope display.
    pub fn ink_core(self) -> Color32 {
        match self {
            Self::Project => Proto::COPPER,
            Self::Hubs => Proto::COPPER,
            Self::Points => Proto::TEAL,
            Self::Mappings => Proto::INDIGO,
            Self::Traffic => Proto::SAGE,
            Self::Streams => Proto::STREAMS,
            Self::Modbus => Proto::MODBUS,
            Self::Mqtt => Proto::MQTT,
            Self::OpcUa => Proto::OPC,
            Self::Files => Proto::FILES,
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
            Self::Mappings => Proto::INDIGO_DEEP,
            Self::Traffic => Proto::SAGE_DEEP,
            Self::Streams => Proto::STREAMS_DEEP,
            Self::Modbus => Proto::MODBUS_DEEP,
            Self::Mqtt => Proto::MQTT_DEEP,
            Self::OpcUa => Proto::OPC_DEEP,
            Self::Files => Proto::FILES_DEEP,
        }
    }
}
