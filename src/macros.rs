#[cfg(test)]
#[allow(unused_macros)]
macro_rules! map {
    ($($key:expr => $value:expr),*) => {{
        #[allow(unused_mut)]
        let mut map = ::std::collections::HashMap::new();
        $(map.insert($key.into(), $value.into());)*
        map
    }};
}

macro_rules! time {
    ($hour:expr, $minute:expr) => {
        time!($hour, $minute, 0)
    };
    ($hour:expr, $minute:expr, $second:expr) => {
        chrono::NaiveTime::from_hms_opt($hour, $minute, $second).unwrap()
    };
}

#[cfg(test)]
macro_rules! date {
    ($year:expr, $month:expr, $day:expr) => {
        chrono::NaiveDate::from_ymd_opt($year, $month, $day).unwrap()
    };
}

#[cfg(test)]
macro_rules! date_time {
    ($year:expr, $month:expr, $day:expr) => {
        date_time!($year, $month, $day, 0, 0, 0)
    };
    ($year:expr, $month:expr, $day:expr, $hour:expr, $minute:expr, $second:expr) => {
        chrono::NaiveDate::from_ymd_opt($year, $month, $day)
            .and_then(|d| d.and_hms_opt($hour, $minute, $second))
            .unwrap()
    };
}
