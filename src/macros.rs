macro_rules! map {
    ($($key:expr => $value:expr),*) => {{
        #[allow(unused_mut)]
        let mut map = ::std::collections::HashMap::new();
        $(map.insert($key.into(), $value.into());)*
        map
    }};
}