//! 单实例互斥测试：同一路径/名称的锁只能被获取一次。

use websocket2serial::platform::single_instance::SingleInstance;

#[test]
fn single_instance_is_exclusive() {
    let p = std::env::temp_dir().join(format!("ws2s-{}.lock", std::process::id()));
    let a = SingleInstance::acquire_at(&p).unwrap();
    assert!(a.is_some());
    let b = SingleInstance::acquire_at(&p).unwrap();
    assert!(b.is_none());
}
