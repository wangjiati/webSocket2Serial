//! 串口参数、传输抽象、真实实现入口与端口扫描。

pub mod params;
pub mod real;
pub mod scan;
pub mod transport;

pub use params::{ParamError, SerialParams};
pub use real::{map_io_error, RealTransport, SystemTransportFactory};
pub use scan::{PortDiff, PortScanner, SystemScanner};
pub use transport::{SerialTransport, TransportError, TransportFactory};
