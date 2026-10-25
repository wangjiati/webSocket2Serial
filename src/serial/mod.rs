//! 串口参数、传输抽象、真实实现入口与端口扫描。

pub mod params;
pub mod scan;
pub mod transport;

pub use params::{ParamError, SerialParams};
pub use scan::{PortDiff, PortScanner, SystemScanner};
pub use transport::{SerialTransport, TransportError, TransportFactory};
