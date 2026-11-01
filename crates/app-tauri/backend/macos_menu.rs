use tauri::menu::{Menu, MenuItemKind};

pub(super) fn localize<R: tauri::Runtime>(menu: &Menu<R>) -> tauri::Result<()> {
    let labels: [(&str, &[Option<&str>]); 6] = [
        (
            "Husk",
            &[
                Some("关于 Husk"),
                None,
                Some("服务"),
                None,
                Some("隐藏 Husk"),
                Some("隐藏其他"),
                None,
                Some("退出 Husk"),
            ],
        ),
        ("文件", &[Some("关闭窗口")]),
        (
            "编辑",
            &[
                Some("撤销"),
                Some("重做"),
                None,
                Some("剪切"),
                Some("拷贝"),
                Some("粘贴"),
                Some("全选"),
            ],
        ),
        ("显示", &[Some("切换全屏幕")]),
        (
            "窗口",
            &[Some("最小化"), Some("缩放"), None, Some("关闭窗口")],
        ),
        ("帮助", &[]),
    ];
    let invalid_structure = || {
        tauri::Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "unexpected macOS default menu structure",
        ))
    };
    let roots = menu.items()?;
    if roots.len() != labels.len() {
        return Err(invalid_structure());
    }
    // Positions follow Tauri's default menu; application names can vary by bundle.
    for (root, (_, children)) in roots.iter().zip(labels.iter()) {
        let MenuItemKind::Submenu(submenu) = root else {
            return Err(invalid_structure());
        };
        let items = submenu.items()?;
        if items.len() != children.len()
            || items
                .iter()
                .any(|item| !matches!(item, MenuItemKind::Predefined(_)))
        {
            return Err(invalid_structure());
        }
    }
    for (root, (title, children)) in roots.iter().zip(labels.iter()) {
        let MenuItemKind::Submenu(submenu) = root else {
            return Err(invalid_structure());
        };
        submenu.set_text(title)?;
        for (item, title) in submenu.items()?.iter().zip(children.iter()) {
            let MenuItemKind::Predefined(item) = item else {
                return Err(invalid_structure());
            };
            if let Some(title) = title {
                item.set_text(title)?;
            }
        }
    }
    Ok(())
}
