use tray_icon::menu::{Menu, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

pub struct Tray {
    _icon: TrayIcon,
    pub capture_id: MenuId,
    pub main_id: MenuId,
    pub quit_id: MenuId,
}

pub fn build() -> Result<Tray, Box<dyn std::error::Error>> {
    let menu = Menu::new();
    let capture = MenuItem::new("Snabbanteckning", true, None);
    let main = MenuItem::new("Öppna Omni", true, None);
    let quit = MenuItem::new("Avsluta", true, None);
    menu.append_items(&[&capture, &main, &PredefinedMenuItem::separator(), &quit])?;
    let icon = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip(crate::app::TITLE)
        .with_icon(make_icon()?)
        .build()?;
    Ok(Tray {
        _icon: icon,
        capture_id: capture.id().clone(),
        main_id: main.id().clone(),
        quit_id: quit.id().clone(),
    })
}

/// 32×32 procedural icon: rounded teal square with a white "+".
fn make_icon() -> Result<Icon, tray_icon::BadIcon> {
    const S: i32 = 32;
    let mut px = Vec::with_capacity((S * S * 4) as usize);
    for y in 0..S {
        for x in 0..S {
            let (cx, cy) = ((x - S / 2).abs(), (y - S / 2).abs());
            let corner = cx > 11 && cy > 11 && (cx - 11).pow(2) + (cy - 11).pow(2) > 25;
            let inside = cx <= 15 && cy <= 15 && !corner;
            let plus = (cx <= 2 && cy <= 9) || (cy <= 2 && cx <= 9);
            let c: [u8; 4] = if !inside {
                [0, 0, 0, 0]
            } else if plus {
                [255, 255, 255, 255]
            } else {
                [38, 166, 154, 255]
            };
            px.extend_from_slice(&c);
        }
    }
    Icon::from_rgba(px, S as u32, S as u32)
}
