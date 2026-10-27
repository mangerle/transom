//! 凭据模板（Identity）管理模块：清单、新建/编辑、删除与密钥环读写。

mod ops;
#[cfg(test)]
mod tests;
mod types;
mod view;

pub(crate) use types::IdentityEditState;
