#[macro_export]
macro_rules! format_ptr {
    ($str:literal $($args:tt)*) => {
        json_patch_ext::PointerBuf::parse(&format!($str $($args)*)).expect("pointer parse error")
    };
}
