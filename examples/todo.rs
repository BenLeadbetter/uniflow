use futures::StreamExt;
use std::sync::{Arc, Mutex};
use uniflow::{Get, Watch};

#[derive(Clone, Default, Debug, PartialEq)]
struct Item {
    what: String,
    done: bool,
}

impl From<Item> for ratatui::prelude::Text<'_> {
    fn from(value: Item) -> Self {
        Self::raw(format!(
            "[{}] {}",
            if value.done { "X" } else { " " },
            value.what
        ))
    }
}

#[derive(Clone, Default, Debug, PartialEq)]
enum Focus {
    Item(usize),
    #[default]
    Editor,
}

#[derive(Clone, Default, Debug, PartialEq)]
struct ToDo {
    items: Vec<Item>,
    focus: Focus,
    edit_text: String,
}

enum Action {
    Add,
    Toggle(usize),
    FocusUp,
    FocusDown,
    Type(char),
    Backspace,
}

fn reducer(mut state: ToDo, action: Action) -> ToDo {
    use Action::*;
    match action {
        Add => {
            state.items.push(Item {
                what: std::mem::take(&mut state.edit_text),
                done: false,
            });
        }
        Toggle(index) => {
            state.items[index].done = !state.items[index].done;
        }
        FocusUp => {
            state.focus = match state.focus {
                Focus::Editor => {
                    if state.items.is_empty() {
                        Focus::Editor
                    } else {
                        Focus::Item(state.items.len() - 1)
                    }
                }
                Focus::Item(index) => {
                    if index == 0 {
                        Focus::Editor
                    } else {
                        Focus::Item(index - 1)
                    }
                }
            }
        }
        FocusDown => {
            state.focus = match state.focus {
                Focus::Editor => {
                    if state.items.is_empty() {
                        Focus::Editor
                    } else {
                        Focus::Item(0)
                    }
                }
                Focus::Item(index) => {
                    if index == state.items.len() - 1 {
                        Focus::Editor
                    } else {
                        Focus::Item(index + 1)
                    }
                }
            }
        }
        Type(c) => {
            if matches!(state.focus, Focus::Editor) {
                state.edit_text = format!("{}{}", state.edit_text, c);
            }
        }
        Backspace => {
            if matches!(state.focus, Focus::Editor) && !state.edit_text.is_empty() {
                state.edit_text = state.edit_text[..state.edit_text.len() - 1].into();
            }
        }
    }
    state
}

struct Teardown;

impl std::ops::Drop for Teardown {
    fn drop(&mut self) {
        ratatui::restore();
    }
}

#[tokio::main]
async fn main() {
    uniflow::any_spawner::Executor::init_tokio().expect("initialize tokio executor");
    let store = std::sync::Arc::new(uniflow::Store::new(ToDo::default(), reducer));
    let terminal = Arc::new(Mutex::new(ratatui::init()));
    let _teardown = Teardown;

    // initial render
    terminal
        .lock()
        .unwrap()
        .draw(|frame| {
            render(frame, store.get());
        })
        .unwrap();

    // render on state changes
    {
        let watch_terminal = terminal.clone();
        let render_store = store.clone();
        store.watch(move |_state| {
            watch_terminal
                .lock()
                .unwrap()
                .draw(|frame| {
                    render(frame, render_store.get());
                })
                .unwrap();
        });
    }

    // map events to actions
    {
        let mut stream = crossterm::event::EventStream::new();
        while let Some(Ok(event)) = stream.next().await {
            let state = store.get();
            use crossterm::event::{Event, KeyCode, KeyEvent};
            if let Event::Key(event) = event {
                match event {
                    KeyEvent {
                        code: KeyCode::Up, ..
                    } => {
                        store.dispatch(Action::FocusUp);
                    }
                    KeyEvent {
                        code: KeyCode::Down,
                        ..
                    } => {
                        store.dispatch(Action::FocusDown);
                    }
                    KeyEvent {
                        code: KeyCode::Enter,
                        ..
                    } => match state.focus {
                        Focus::Editor => {
                            if !state.edit_text.is_empty() {
                                store.dispatch(Action::Add)
                            }
                        }
                        Focus::Item(index) => store.dispatch(Action::Toggle(index)),
                    },
                    KeyEvent {
                        code: KeyCode::Char(c),
                        ..
                    } => match state.focus {
                        Focus::Editor => {
                            store.dispatch(Action::Type(c));
                        }
                        Focus::Item(index) => {
                            if c == ' ' {
                                store.dispatch(Action::Toggle(index));
                            } else if c == 'j' {
                                store.dispatch(Action::FocusDown);
                            } else if c == 'k' {
                                store.dispatch(Action::FocusUp);
                            }
                        }
                    },
                    KeyEvent {
                        code: KeyCode::Backspace,
                        ..
                    } => {
                        if let Focus::Editor = state.focus {
                            store.dispatch(Action::Backspace);
                        }
                    }
                    KeyEvent {
                        code: KeyCode::Esc, ..
                    } => break,
                    _ => {}
                }
            };
        }
    }
}

fn render(frame: &mut ratatui::Frame, state: ToDo) {
    let [title, list, edit] = ratatui::layout::Layout::vertical([
        ratatui::layout::Constraint::Length(3),
        ratatui::layout::Constraint::Min(3),
        ratatui::layout::Constraint::Length(3),
    ])
    .areas(frame.area());
    frame.render_widget(
        ratatui::widgets::Paragraph::new("To Do")
            .alignment(ratatui::layout::Alignment::Center)
            .block(ratatui::widgets::Block::default().borders(ratatui::widgets::Borders::ALL)),
        title,
    );
    frame.render_widget(
        ratatui::widgets::List::new(state.items.into_iter().enumerate().map(|(i, item)| {
            ratatui::widgets::ListItem::from(item).style(ratatui::prelude::Style::default().fg(
                match state.focus {
                    Focus::Item(index) if index == i => ratatui::prelude::Color::Yellow,
                    _ => ratatui::prelude::Color::White,
                },
            ))
        }))
        .block(ratatui::widgets::Block::default().borders(ratatui::widgets::Borders::ALL)),
        list,
    );
    frame.render_widget(
        ratatui::widgets::Paragraph::new(format!("> {}", state.edit_text))
            .block(ratatui::widgets::Block::default().borders(ratatui::widgets::Borders::ALL))
            .style(ratatui::prelude::Style::default().fg(
                if matches!(state.focus, Focus::Editor) {
                    ratatui::prelude::Color::Yellow
                } else {
                    ratatui::prelude::Color::White
                },
            )),
        edit,
    );
}
