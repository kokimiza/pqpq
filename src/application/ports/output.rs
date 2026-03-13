pub mod combat_output_port;
pub mod host_ring_output_port;
pub mod join_ring_output_port;
pub mod list_rings_output_port;

pub use combat_output_port::CombatOutputPort;
pub use host_ring_output_port::HostRingOutputPort;
pub use join_ring_output_port::JoinRingOutputPort;
pub use list_rings_output_port::ListRingsOutputPort;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum ErrorLayer {
    Domain,
    Application,
    Infrastructure,
}
