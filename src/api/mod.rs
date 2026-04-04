//! API 数据传输对象（DTO）
//!
//! 集中管理所有请求、响应和查询参数类型。
//! handler 层只做路由转发，service 层只做业务逻辑，
//! DTO 定义统一放在这里，避免各层之间的类型耦合。

pub mod query;
pub mod request;
pub mod response;
