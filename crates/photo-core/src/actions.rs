//! Stable action catalogue shared by keyboard, palettes and API clients.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionId {
    RotateLeft,
    RotateRight,
    FlipHorizontal,
    FlipVertical,
    EnterAlbum,
    ResizeImage,
    ConvertImage,
    Search,
    Settings,
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
    CopyVideoFrame,
    SaveVideoFrame,
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
    add(Settings, "Настройки…", "Приложение", "Ctrl ,");
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
    if context.space
        && context.has_gallery
        && context.targets > 0
        && !context.video
        && !context.busy
    {
        add(RotateLeft, "Повернуть влево", "Изображение", "Q");
        add(RotateRight, "Повернуть вправо", "Изображение", "E");
        add(FlipHorizontal, "Отразить по горизонтали", "Изображение", "");
        add(FlipVertical, "Отразить по вертикали", "Изображение", "");
        add(ResizeImage, "Размер и вес…", "Изображение", "");
        add(ConvertImage, "Конвертировать формат…", "Изображение", "");
    }
    if context.space && context.video && context.targets > 0 && !context.busy {
        add(CopyVideoFrame, "Копировать текущий кадр", "Видео", "");
        add(SaveVideoFrame, "Сохранить текущий кадр PNG", "Видео", "");
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
    fn settings_remain_discoverable_without_gallery_and_while_busy() {
        let actions = available(&Context {
            album_overview: false,
            space: false,
            targets: 0,
            video: false,
            busy: true,
            has_gallery: false,
            marked: false,
        });
        let settings = actions.iter().find(|a| a.id == ActionId::Settings).unwrap();
        assert_eq!(settings.shortcut, "Ctrl ,");
    }

    #[test]
    fn transforms_require_an_available_still_image_in_space() {
        let transforms = [
            ActionId::RotateLeft,
            ActionId::RotateRight,
            ActionId::FlipHorizontal,
            ActionId::FlipVertical,
        ];
        let mut context = Context {
            album_overview: false,
            space: true,
            targets: 1,
            video: false,
            busy: false,
            has_gallery: true,
            marked: false,
        };
        assert!(
            transforms
                .iter()
                .all(|id| available(&context).iter().any(|a| a.id == *id))
        );
        for invalid in 0..5 {
            context.space = invalid != 0;
            context.targets = usize::from(invalid != 1);
            context.video = invalid == 2;
            context.busy = invalid == 3;
            context.has_gallery = invalid != 4;
            assert!(
                available(&context)
                    .iter()
                    .all(|a| !transforms.contains(&a.id))
            );
        }
    }
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

#[cfg(test)]
mod video_frame_tests {
    use super::*;
    #[test]
    fn video_frame_commands_require_space_video_and_idle_target() {
        for space in [false, true] {
            for video in [false, true] {
                for busy in [false, true] {
                    for targets in [0, 1] {
                        let context = Context {
                            album_overview: false,
                            space,
                            video,
                            busy,
                            targets,
                            has_gallery: true,
                            marked: false,
                        };
                        for id in [ActionId::CopyVideoFrame, ActionId::SaveVideoFrame] {
                            let item = available(&context).into_iter().find(|a| a.id == id);
                            assert_eq!(item.is_some(), space && video && !busy && targets > 0);
                            if let Some(item) = item {
                                assert!(item.shortcut.is_empty());
                            }
                        }
                    }
                }
            }
        }
    }
}
