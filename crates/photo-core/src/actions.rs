//! Stable action catalogue shared by keyboard, palettes and API clients.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionId {
    EnterAlbum,
    ResizeImage,
    ConvertImage,
    Search,
    Delete,
    Duplicate,
    Copy,
    Paste,
    Rename,
    Open,
    Back,
    Next,
    Previous,
    ClearMarks,
    Refresh,
    Fit,
    PlayPause,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionInfo {
    pub id: ActionId,
    pub label: String,
    pub group: String,
    pub shortcut: String,
}
pub struct Context {
    pub album_overview: bool,
    pub space: bool,
    pub targets: usize,
    pub video: bool,
    pub busy: bool,
    pub has_gallery: bool,
    pub marked: bool,
}
pub fn available(context: &Context) -> Vec<ActionInfo> {
    use ActionId::*;
    let mut actions = Vec::new();
    let mut add = |id, label: &str, group: &str, shortcut: &str| {
        actions.push(ActionInfo {
            id,
            label: label.into(),
            group: group.into(),
            shortcut: shortcut.into(),
        })
    };
    add(Search, "Поиск Everything", "Навигация", "Ctrl Shift K");
    if context.targets > 0 && !context.busy {
        let copy_label = if context.targets > 1 {
            format!("Копировать файлы ({})", context.targets)
        } else {
            "Копировать".into()
        };
        add(Copy, &copy_label, "Файлы", "Ctrl C");
        add(Duplicate, "Создать дубликат", "Файлы", "Ctrl D");
        if context.targets == 1 {
            add(Rename, "Переименовать", "Файлы", "F2");
        }
    }
    if context.has_gallery && !context.busy {
        add(Paste, "Вставить в папку", "Буфер обмена", "Ctrl V");
    }
    if context.space && !context.video && !context.busy {
        add(ResizeImage, "Размер и вес…", "Изображение", "");
        add(ConvertImage, "Конвертировать формат…", "Изображение", "");
    }
    if context.space {
        if context.video {
            add(PlayPause, "Воспроизведение / пауза", "Видео", "P");
        } else {
            add(Fit, "Вписать в окно", "Просмотр", "");
        }
        add(Next, "Следующий файл", "Навигация", "→");
        add(Previous, "Предыдущий файл", "Навигация", "←");
        add(Back, "Вернуться в галерею", "Навигация", "Esc / Space");
    } else {
        if context.album_overview {
            add(EnterAlbum, "Войти в альбом", "Навигация", "Enter");
        }
        if context.targets > 0 {
            add(
                Open,
                "Открыть текущий файл",
                "Навигация",
                if context.album_overview {
                    "Space"
                } else {
                    "Enter / Space"
                },
            );
        }
    }
    if !context.space && context.marked {
        add(ClearMarks, "Снять выделение", "Выделение", "Esc");
    }
    if context.has_gallery {
        add(Refresh, "Обновить папку", "Папка", "F5");
    }
    if context.targets > 0 && !context.busy {
        add(Delete, "Удалить", "Удаление", "Delete");
    }
    actions
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn palettes_respect_scope_selection_busy_and_video() {
        let mut context = Context {
            album_overview: false,
            space: false,
            targets: 2,
            video: false,
            busy: false,
            has_gallery: true,
            marked: true,
        };
        let ids = |c: &Context| available(c).into_iter().map(|a| a.id).collect::<Vec<_>>();
        assert!(ids(&context).contains(&ActionId::Duplicate));
        assert!(!ids(&context).contains(&ActionId::Back));
        context.space = true;
        context.video = true;
        assert!(ids(&context).contains(&ActionId::PlayPause));
        assert!(!ids(&context).contains(&ActionId::Fit));
        assert!(!ids(&context).contains(&ActionId::ClearMarks));
        assert_eq!(ids(&context).last(), Some(&ActionId::Delete));
        context.busy = true;
        assert!(!ids(&context).contains(&ActionId::Delete));
        assert!(!ids(&context).contains(&ActionId::Duplicate));
        assert!(ids(&context).contains(&ActionId::Back));
    }
}
