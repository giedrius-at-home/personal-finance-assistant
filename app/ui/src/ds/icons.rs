//! Lucide icons (ISC licence, lucide.dev) as inline SVG, generated from
//! lucide-react 1.47 so the app needs no icon font or network.

use leptos::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Activity,
    ArrowDownLeft,
    ArrowLeft,
    ArrowRight,
    ArrowUpRight,
    CalendarCheck,
    CalendarDays,
    ChartNoAxesCombined,
    Check,
    ChevronDown,
    ChevronLeft,
    ChevronRight,
    ChevronUp,
    CircleAlert,
    CircleCheck,
    Database,
    ExternalLink,
    Feather,
    Gauge,
    House,
    ImageOff,
    Inbox,
    Info,
    Link2,
    Pencil,
    Plus,
    Minus,
    Power,
    RefreshCw,
    Search,
    ShieldCheck,
    Shirt,
    SlidersHorizontal,
    Sparkles,
    Trash2,
    TrendingDown,
    TrendingUp,
    Undo2,
    Upload,
    X,
    LayoutDashboard,
    ArrowLeftRight,
    ChartPie,
    CalendarClock,
    Settings,
    Wallet,
    Landmark,
    Tag,
    Receipt,
    Lock,
    Clock,
    Loader,
    EyeOff,
    Eye,
    Filter,
    Download,
    Copy,
    PiggyBank,
    Repeat,
    CircleDollarSign,
    Sun,
    Moon,
    Split,
    Scissors,
    Users,
    Banknote,
    CreditCard,
    Flag,
    ListChecks,
    Layers,
    FolderTree,
    Ellipsis,
    Hourglass,
    BadgeCheck,
    CircleX,
    TriangleAlert,
    History,
    GripVertical,
    ArrowUp,
    ArrowDown,
}

impl Icon {
    fn body(self) -> &'static str {
        match self {
            Self::Activity => r#""#,
            Self::ArrowDownLeft => r#"<path d="M17 7 7 17"/><path d="M17 17H7V7"/>"#,
            Self::ArrowLeft => r#"<path d="m12 19-7-7 7-7"/><path d="M19 12H5"/>"#,
            Self::ArrowRight => r#"<path d="M5 12h14"/><path d="m12 5 7 7-7 7"/>"#,
            Self::ArrowUpRight => r#"<path d="M7 7h10v10"/><path d="M7 17 17 7"/>"#,
            Self::CalendarCheck => {
                r#"<path d="M8 2v3"/><path d="M16 2v3"/><rect x="3" y="3" width="18" height="18" rx="2"/><path d="M3 9h18"/><path d="m9 15 2 2 4-4"/>"#
            }
            Self::CalendarDays => {
                r#"<path d="M8 2v3"/><path d="M16 2v3"/><rect x="3" y="3" width="18" height="18" rx="2"/><path d="M3 9h18"/><path d="M8 13h.01"/><path d="M12 13h.01"/><path d="M16 13h.01"/><path d="M8 17h.01"/><path d="M12 17h.01"/><path d="M16 17h.01"/>"#
            }
            Self::ChartNoAxesCombined => {
                r#"<path d="M12 16v5"/><path d="M16 14.639V21"/><path d="M20 10.656V21"/><path d="M4 18.463V21"/><path d="M8 14.656V21"/>"#
            }
            Self::Check => r#"<path d="M20 6 9 17l-5-5"/>"#,
            Self::ChevronDown => r#"<path d="m6 9 6 6 6-6"/>"#,
            Self::ChevronLeft => r#"<path d="m15 18-6-6 6-6"/>"#,
            Self::ChevronRight => r#"<path d="m9 18 6-6-6-6"/>"#,
            Self::ChevronUp => r#"<path d="m18 15-6-6-6 6"/>"#,
            Self::CircleAlert => {
                r#"<circle cx="12" cy="12" r="10"/><line x1="12" x2="12" y1="8" y2="12"/><line x1="12" x2="12.01" y1="16" y2="16"/>"#
            }
            Self::CircleCheck => {
                r#"<circle cx="12" cy="12" r="10"/><path d="m16 9-5.5 5.5L8 12"/>"#
            }
            Self::Database => {
                r#"<ellipse cx="12" cy="5" rx="9" ry="3"/><path d="M3 5V19A9 3 0 0 0 21 19V5"/><path d="M3 12A9 3 0 0 0 21 12"/>"#
            }
            Self::ExternalLink => {
                r#"<path d="M15 3h6v6"/><path d="M10 14 21 3"/><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/>"#
            }
            Self::Feather => r#"<path d="M16 8 2 22"/><path d="M17.488 15H9"/>"#,
            Self::Gauge => r#"<path d="m12 14 4-4"/><path d="M3.34 19a10 10 0 1 1 17.32 0"/>"#,
            Self::House => r#"<path d="M15 21v-8a1 1 0 0 0-1-1h-4a1 1 0 0 0-1 1v8"/>"#,
            Self::ImageOff => {
                r#"<line x1="2" x2="22" y1="2" y2="22"/><path d="M10.41 10.41a2 2 0 1 1-2.83-2.83"/><line x1="13.5" x2="6" y1="13.5" y2="21"/><line x1="18" x2="21" y1="12" y2="15"/><path d="M21 15V5a2 2 0 0 0-2-2H9"/>"#
            }
            Self::Inbox => r#"<polyline points="22 12 16 12 14 15 10 15 8 12 2 12"/>"#,
            Self::Info => {
                r#"<circle cx="12" cy="12" r="10"/><path d="M12 16v-4"/><path d="M12 8h.01"/>"#
            }
            Self::Link2 => {
                r#"<path d="M9 17H7A5 5 0 0 1 7 7h2"/><path d="M15 7h2a5 5 0 1 1 0 10h-2"/><line x1="8" x2="16" y1="12" y2="12"/>"#
            }
            Self::Pencil => r#"<path d="m15 5 4 4"/>"#,
            Self::Plus => r#"<path d="M5 12h14"/><path d="M12 5v14"/>"#,
            Self::Minus => r#"<path d="M5 12h14"/>"#,
            Self::Power => r#"<path d="M12 2v10"/><path d="M18.4 6.6a9 9 0 1 1-12.77.04"/>"#,
            Self::RefreshCw => {
                r#"<path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/><path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16"/><path d="M8 16H3v5"/>"#
            }
            Self::Search => r#"<path d="m21 21-4.34-4.34"/><circle cx="11" cy="11" r="8"/>"#,
            Self::ShieldCheck => r#"<path d="m9 12 2 2 4-4"/>"#,
            Self::Shirt => r#""#,
            Self::SlidersHorizontal => {
                r#"<path d="M10 5H3"/><path d="M12 19H3"/><path d="M14 3v4"/><path d="M16 17v4"/><path d="M21 12h-9"/><path d="M21 19h-5"/><path d="M21 5h-7"/><path d="M8 10v4"/><path d="M8 12H3"/>"#
            }
            Self::Sparkles => {
                r#"<path d="M20 2v4"/><path d="M22 4h-4"/><circle cx="4" cy="20" r="2"/>"#
            }
            Self::Trash2 => {
                r#"<path d="M10 11v6"/><path d="M14 11v6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"/><path d="M3 6h18"/><path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/>"#
            }
            Self::TrendingDown => r#"<path d="M16 17h6v-6"/><path d="m22 17-8.5-8.5-5 5L2 7"/>"#,
            Self::TrendingUp => r#"<path d="M16 7h6v6"/><path d="m22 7-8.5 8.5-5-5L2 17"/>"#,
            Self::Undo2 => {
                r#"<path d="M9 14 4 9l5-5"/><path d="M4 9h10.5a5.5 5.5 0 0 1 5.5 5.5a5.5 5.5 0 0 1-5.5 5.5H11"/>"#
            }
            Self::Upload => {
                r#"<path d="M12 3v12"/><path d="m17 8-5-5-5 5"/><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/>"#
            }
            Self::X => r#"<path d="M18 6 6 18"/><path d="m6 6 12 12"/>"#,
            Self::LayoutDashboard => {
                r#"<rect width="7" height="9" x="3" y="3" rx="1"/><rect width="7" height="5" x="14" y="3" rx="1"/><rect width="7" height="9" x="14" y="12" rx="1"/><rect width="7" height="5" x="3" y="16" rx="1"/>"#
            }
            Self::ArrowLeftRight => {
                r#"<path d="M8 3 4 7l4 4"/><path d="M4 7h16"/><path d="m16 21 4-4-4-4"/><path d="M20 17H4"/>"#
            }
            Self::ChartPie => r#"<path d="M21.21 15.89A10 10 0 1 1 8 2.83"/>"#,
            Self::CalendarClock => {
                r#"<path d="M16 14v2.2l1.6 1"/><path d="M16 2v3"/><path d="M21 7.338V5a2 2 0 00-2-2H5a2 2 0 00-2 2v14a2 2 0 002 2h2.338"/><path d="M3 9h5.859"/><path d="M8 2v3"/><circle cx="16" cy="16" r="6"/>"#
            }
            Self::Settings => r#"<circle cx="12" cy="12" r="3"/>"#,
            Self::Wallet => r#"<path d="M3 5v14a2 2 0 0 0 2 2h15a1 1 0 0 0 1-1v-4"/>"#,
            Self::Landmark => {
                r#"<path d="M10 18v-7"/><path d="M14 18v-7"/><path d="M18 18v-7"/><path d="M3 22h18"/><path d="M6 18v-7"/>"#
            }
            Self::Tag => r#"<circle cx="7.5" cy="7.5" r=".5" fill="currentColor"/>"#,
            Self::Receipt => {
                r#"<path d="M12 17V7"/><path d="M16 8h-6a2 2 0 0 0 0 4h4a2 2 0 0 1 0 4H8"/>"#
            }
            Self::Lock => {
                r#"<rect width="18" height="11" x="3" y="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/>"#
            }
            Self::Clock => r#"<circle cx="12" cy="12" r="10"/><path d="M12 6v6l4 2"/>"#,
            Self::Loader => {
                r#"<path d="M12 2v4"/><path d="m16.2 7.8 2.9-2.9"/><path d="M18 12h4"/><path d="m16.2 16.2 2.9 2.9"/><path d="M12 18v4"/><path d="m4.9 19.1 2.9-2.9"/><path d="M2 12h4"/><path d="m4.9 4.9 2.9 2.9"/>"#
            }
            Self::EyeOff => {
                r#"<path d="M14.084 14.158a3 3 0 0 1-4.242-4.242"/><path d="m2 2 20 20"/>"#
            }
            Self::Eye => r#"<circle cx="12" cy="12" r="3"/>"#,
            Self::Filter => r#""#,
            Self::Download => {
                r#"<path d="M12 15V3"/><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><path d="m7 10 5 5 5-5"/>"#
            }
            Self::Copy => {
                r#"<rect width="14" height="14" x="8" y="8" rx="2" ry="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/>"#
            }
            Self::PiggyBank => r#"<path d="M16 10h.01"/><path d="M2 8v1a2 2 0 0 0 2 2h1"/>"#,
            Self::Repeat => {
                r#"<path d="m17 2 4 4-4 4"/><path d="M3 11v-1a4 4 0 0 1 4-4h14"/><path d="m7 22-4-4 4-4"/><path d="M21 13v1a4 4 0 0 1-4 4H3"/>"#
            }
            Self::CircleDollarSign => {
                r#"<circle cx="12" cy="12" r="10"/><path d="M16 8h-6a2 2 0 1 0 0 4h4a2 2 0 1 1 0 4H8"/><path d="M12 18V6"/>"#
            }
            Self::Sun => {
                r#"<circle cx="12" cy="12" r="4"/><path d="M12 2v2"/><path d="M12 20v2"/><path d="m4.93 4.93 1.41 1.41"/><path d="m17.66 17.66 1.41 1.41"/><path d="M2 12h2"/><path d="M20 12h2"/><path d="m6.34 17.66-1.41 1.41"/><path d="m19.07 4.93-1.41 1.41"/>"#
            }
            Self::Moon => r#""#,
            Self::Split => {
                r#"<path d="M16 3h5v5"/><path d="M8 3H3v5"/><path d="M12 22v-8.3a4 4 0 0 0-1.172-2.872L3 3"/><path d="m15 9 6-6"/>"#
            }
            Self::Scissors => {
                r#"<circle cx="6" cy="6" r="3"/><path d="M8.12 8.12 12 12"/><path d="M20 4 8.12 15.88"/><circle cx="6" cy="18" r="3"/><path d="M14.8 14.8 20 20"/>"#
            }
            Self::Users => {
                r#"<path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><path d="M16 3.128a4 4 0 0 1 0 7.744"/><path d="M22 21v-2a4 4 0 0 0-3-3.87"/><circle cx="9" cy="7" r="4"/>"#
            }
            Self::Banknote => {
                r#"<rect width="20" height="12" x="2" y="6" rx="2"/><circle cx="12" cy="12" r="2"/><path d="M6 12h.01M18 12h.01"/>"#
            }
            Self::CreditCard => {
                r#"<rect width="20" height="14" x="2" y="5" rx="2"/><line x1="2" x2="22" y1="10" y2="10"/><path d="M6 14h2"/>"#
            }
            Self::Flag => r#""#,
            Self::ListChecks => {
                r#"<path d="M13 5h8"/><path d="M13 12h8"/><path d="M13 19h8"/><path d="m3 17 2 2 4-4"/><path d="m3 7 2 2 4-4"/>"#
            }
            Self::Layers => r#""#,
            Self::FolderTree => {
                r#"<path d="M3 5a2 2 0 0 0 2 2h3"/><path d="M3 3v13a2 2 0 0 0 2 2h3"/>"#
            }
            Self::Ellipsis => {
                r#"<circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/><circle cx="5" cy="12" r="1"/>"#
            }
            Self::Hourglass => r#"<path d="M5 22h14"/><path d="M5 2h14"/>"#,
            Self::BadgeCheck => r#"<path d="m16 9-5.5 5.5L8 12"/>"#,
            Self::CircleX => {
                r#"<circle cx="12" cy="12" r="10"/><path d="m15 9-6 6"/><path d="m9 9 6 6"/>"#
            }
            Self::TriangleAlert => r#"<path d="M12 9v4"/><path d="M12 17h.01"/>"#,
            Self::GripVertical => {
                r#"<circle cx="9" cy="12" r="1"/><circle cx="9" cy="5" r="1"/><circle cx="9" cy="19" r="1"/><circle cx="15" cy="12" r="1"/><circle cx="15" cy="5" r="1"/><circle cx="15" cy="19" r="1"/>"#
            }
            Self::ArrowUp => r#"<path d="m5 12 7-7 7 7"/><path d="M12 19V5"/>"#,
            Self::ArrowDown => r#"<path d="M12 5v14"/><path d="m19 12-7 7-7-7"/>"#,
            Self::History => {
                r#"<path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/><path d="M12 7v5l4 2"/>"#
            }
        }
    }
}

/// An icon at `size` px in the current text colour, hidden from assistive tech.
#[component]
pub fn Lucide(
    icon: Icon,
    #[prop(default = 16)] size: u32,
    #[prop(optional, into)] class: String,
    #[prop(default = 2.0)] stroke: f32,
) -> impl IntoView {
    view! {
        <svg
            xmlns="http://www.w3.org/2000/svg"
            width=size
            height=size
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width=stroke
            stroke-linecap="round"
            stroke-linejoin="round"
            class=format!("lucide shrink-0 {class}")
            aria-hidden="true"
            inner_html=icon.body()
        ></svg>
    }
}
