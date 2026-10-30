use super::icons::Keyline;
use chrono::Timelike;

pub fn time_greeting(dark: bool) -> (&'static str, Keyline, u32) {
    let hour = chrono::Local::now().hour();
    match hour {
        0..=4 => (
            "夜深了",
            Keyline::MoonStar,
            if dark { 0xa5b4fc } else { 0x818cf8 },
        ),
        5..=8 => (
            "早上好",
            Keyline::Sunrise,
            if dark { 0xfbbf24 } else { 0xf59e0b },
        ),
        9..=11 => (
            "上午好",
            Keyline::Sun,
            if dark { 0xfbbf24 } else { 0xf59e0b },
        ),
        12..=13 => (
            "中午好",
            Keyline::SunMedium,
            if dark { 0xfb923c } else { 0xf97316 },
        ),
        14..=17 => (
            "下午好",
            Keyline::SunDim,
            if dark { 0xf59e0b } else { 0xd97706 },
        ),
        18..=19 => (
            "傍晚好",
            Keyline::Sunset,
            if dark { 0xfb923c } else { 0xf97316 },
        ),
        _ => (
            "晚上好",
            Keyline::Moon,
            if dark { 0xa5b4fc } else { 0x818cf8 },
        ),
    }
}
