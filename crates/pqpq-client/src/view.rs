//! Ratatui drawing. Reads the app state only; never changes the game.

use std::time::Instant;

use pqpq_protocol::{CarStatus, PlayerId, RoomView};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::canvas::{Canvas, Line as Segment};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app::{App, CarView, Screen};

pub const MIN_WIDTH: u16 = 80;
pub const MIN_HEIGHT: u16 = 24;
const SIDE_PANEL: u16 = 26;
const PALETTE: [Color; 8] = [
    Color::Cyan,
    Color::Yellow,
    Color::Magenta,
    Color::Green,
    Color::LightRed,
    Color::LightBlue,
    Color::White,
    Color::LightGreen,
];

pub fn draw(f: &mut Frame, app: &App, now: Instant) {
    let area = f.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        let msg = format!(
            "端末を {MIN_WIDTH}×{MIN_HEIGHT} 以上に広げてください（現在 {}×{}）\n操作は一時的に無効です。q で終了",
            area.width, area.height
        );
        f.render_widget(
            Paragraph::new(msg)
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true }),
            area,
        );
        return;
    }
    match app.screen() {
        Screen::Connecting => centered(
            f,
            "接続中",
            vec![
                format!("{} に接続しています…", app.cfg.server_addr).into(),
                format!("ROOM {}", app.cfg.room_id).into(),
                "".into(),
                "q 終了".into(),
            ],
        ),
        Screen::Error => centered(
            f,
            "エラー",
            vec![
                Line::from(app.error.clone().unwrap_or_default()),
                "".into(),
                "q 終了".into(),
            ],
        ),
        Screen::Lobby => lobby(f, app),
        Screen::Results => results(f, app),
        Screen::Countdown | Screen::Race | Screen::Spectate => race(f, app, now),
    }
}

fn centered(f: &mut Frame, title: &str, lines: Vec<Line>) {
    let area = f.area();
    let h = (lines.len() as u16 + 2).min(area.height);
    let rect = Rect {
        x: area.width / 8,
        y: (area.height - h) / 2,
        width: area.width * 3 / 4,
        height: h,
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" pqpq · {title} "));
    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true }),
        rect,
    );
}

fn color_of(view: &RoomView, id: PlayerId) -> Color {
    if let Some(index) = view.label_index(id) {
        PALETTE[index % PALETTE.len()]
    } else {
        Color::Gray
    }
}

fn lobby(f: &mut Frame, app: &App) {
    let view = app.view.as_ref().unwrap();
    let racers = view.racers().count();
    let mut lines: Vec<Line> = vec![
        format!("ROOM {}   待機中   {} 人", view.room_id, view.roster.len()).into(),
        "".into(),
    ];
    for e in &view.roster {
        let me = if e.player_id == view.me {
            "  ← あなた"
        } else {
            ""
        };
        let state = if e.ready { "READY" } else { "未Ready" };
        lines.push(
            format!(
                "{:>5}  {}  {}{}",
                e.player_id.to_string(),
                state,
                e.username,
                me
            )
            .into(),
        );
    }
    lines.push("".into());
    lines.push(
        format!(
            "開始条件: {} 人以上・全員 READY（現在 {racers} 人）",
            view.min_racers
        )
        .into(),
    );
    lines.push(notice_line(app));
    lines.push("Enter READY    q 終了".into());
    centered(f, "待機室", lines);
}

fn results(f: &mut Frame, app: &App) {
    let view = app.view.as_ref().unwrap();
    let mut lines: Vec<Line> = vec![format!("ROOM {}   結果", view.room_id).into(), "".into()];
    for (i, r) in view.results.iter().enumerate() {
        let time = match (r.status, r.finish_time_ms) {
            (CarStatus::Finished, Some(ms)) => clock(ms as f64 / 1000.0),
            _ => format!("DNF ({}周)", r.laps),
        };
        let me = if r.player_id == view.me {
            "  ← あなた"
        } else {
            ""
        };
        lines.push(
            format!(
                "{:>2}. {}  {:<12}  {}{}",
                i + 1,
                view.label_of(r.player_id),
                time,
                r.username,
                me
            )
            .into(),
        );
    }
    lines.push("".into());
    lines.push("全員が退出すると部屋は削除され、同じ部屋IDで新しいレースを始められます".into());
    lines.push("q 終了".into());
    centered(f, "結果", lines);
}

fn race(f: &mut Frame, app: &App, now: Instant) {
    let view = app.view.as_ref().unwrap();
    let [header, body, status, help] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(10),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(f.area());
    let [track, side] =
        Layout::horizontal([Constraint::Min(40), Constraint::Length(SIDE_PANEL)]).areas(body);

    let mine = app.my_snapshot();
    let entrants = app.last_snapshot.as_ref().map_or(0, |s| s.entrants);
    let mut head = format!("pqpq  ROOM {}", view.room_id);
    match app.screen() {
        Screen::Countdown => {
            head += &format!("   START IN {:.1}", app.countdown(now).unwrap_or(0.0));
        },
        Screen::Spectate => head += "   観戦中",
        _ => {},
    }
    if let Some(c) = mine {
        let lap = if c.status == CarStatus::Finished {
            "FIN".to_owned()
        } else {
            format!("{}/{}", (c.laps + 1).min(view.laps), view.laps)
        };
        head += &format!("   LAP {lap}   POS {}/{entrants}", c.rank);
    }
    if let Some(rtt) = app.rtt {
        head += &format!("   RTT {}ms", rtt.as_millis());
    }
    f.render_widget(
        Paragraph::new(head).style(Style::new().add_modifier(Modifier::BOLD)),
        header,
    );

    draw_track(f, app, view, &app.cars(now), track);
    standings(f, app, view, side);

    let mut line = Vec::new();
    if let Some(c) = mine {
        let kmh = (c.velocity[0].hypot(c.velocity[1]) * 3.6).round();
        line.push(Span::raw(format!("SPEED {kmh:>3} km/h   ")));
    }
    if let Some(t) = app.race_time(now) {
        line.push(Span::raw(format!("TIME {}   ", clock(t))));
    }
    if app.screen() != Screen::Spectate {
        let label = view.label_of(view.me);
        line.push(Span::styled(
            format!("YOU {label}"),
            Style::new()
                .fg(color_of(view, view.me))
                .add_modifier(Modifier::REVERSED),
        ));
    }
    if app.stale(now) {
        line.push(Span::styled("   通信遅延", Style::new().fg(Color::Red)));
    }
    line.extend(notice_line(app).spans);
    f.render_widget(Paragraph::new(Line::from(line)), status);

    let mut keys = "Up/W 加速  Down/S ブレーキ  Left/Right/A/D 操舵  q 終了".to_owned();
    if !app.keys.release_events {
        keys += "  (互換入力: キーを押し続けてください)";
    }
    f.render_widget(
        Paragraph::new(keys).style(Style::new().fg(Color::DarkGray)),
        help,
    );
}

fn draw_track(f: &mut Frame, app: &App, view: &RoomView, cars: &[CarView], area: Rect) {
    let course = &app.course;
    let [min_x, min_y, max_x, max_y] = course.bounds();
    // Terminal cells are about twice as tall as wide: keep the course's aspect.
    let unit =
        ((max_x - min_x) / area.width as f64).max((max_y - min_y) / (2.0 * area.height as f64));
    let (cx, cy) = ((min_x + max_x) / 2.0, (min_y + max_y) / 2.0);
    let (hw, hh) = (unit * area.width as f64 / 2.0, unit * area.height as f64);
    let (left, right) = course.edges();
    let finish = *course.gates().last().unwrap();
    let canvas = Canvas::default()
        .x_bounds([cx - hw, cx + hw])
        .y_bounds([cy - hh, cy + hh])
        .paint(|ctx| {
            for edge in [&left, &right] {
                for i in 0..edge.len() {
                    let (a, b) = (edge[i], edge[(i + 1) % edge.len()]);
                    ctx.draw(&Segment {
                        x1: a[0],
                        y1: a[1],
                        x2: b[0],
                        y2: b[1],
                        color: Color::Gray,
                    });
                }
            }
            ctx.draw(&Segment {
                x1: finish.a[0],
                y1: finish.a[1],
                x2: finish.b[0],
                y2: finish.b[1],
                color: Color::White,
            });
            ctx.layer();
            for car in cars {
                let mut style = Style::new()
                    .fg(color_of(view, car.id))
                    .add_modifier(Modifier::BOLD);
                if car.me {
                    style = style.add_modifier(Modifier::REVERSED);
                }
                let text = format!("{}{}", car.label, arrow(car.state.direction));
                ctx.print(
                    car.state.position[0],
                    car.state.position[1],
                    Span::styled(text, style),
                );
            }
        });
    f.render_widget(canvas, area);
}

fn standings(f: &mut Frame, app: &App, view: &RoomView, area: Rect) {
    let mut lines = Vec::new();
    if let Some(snap) = &app.last_snapshot {
        let mut cars = snap.cars.clone();
        cars.sort_by_key(|c| c.rank);
        for c in cars {
            let name: String = view
                .name_of(c.player_id)
                .unwrap_or("?")
                .chars()
                .take(10)
                .collect();
            let state = match c.status {
                CarStatus::Finished => "FIN".to_owned(),
                CarStatus::Dnf => "DNF".to_owned(),
                CarStatus::Racing => format!("L{}", (c.laps + 1).min(view.laps)),
            };
            let style = Style::new().fg(color_of(view, c.player_id));
            let style = if c.player_id == view.me {
                style.add_modifier(Modifier::REVERSED)
            } else {
                style
            };
            lines.push(Line::from(Span::styled(
                format!(
                    "{:>2} {} {:<10} {}",
                    c.rank,
                    view.label_of(c.player_id),
                    name,
                    state
                ),
                style,
            )));
        }
    } else if let Some(start) = &view.start {
        for g in &start.grid {
            let name: String = view
                .name_of(g.player_id)
                .unwrap_or("?")
                .chars()
                .take(12)
                .collect();
            lines.push(Line::from(Span::styled(
                format!("   {} {}", view.label_of(g.player_id), name),
                Style::new().fg(color_of(view, g.player_id)),
            )));
        }
    }
    f.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::LEFT).title(" 順位 ")),
        area,
    );
}

fn notice_line(app: &App) -> Line<'static> {
    match &app.notice {
        Some((msg, _)) => Line::from(Span::styled(
            format!("   {msg}"),
            Style::new().fg(Color::Yellow),
        )),
        None => Line::default(),
    }
}

/// ASCII only: arrow glyphs are ambiguous-width in CJK terminals.
fn arrow(direction: f64) -> char {
    let sector = (direction / std::f64::consts::FRAC_PI_4)
        .round()
        .rem_euclid(8.0) as usize;
    ['>', '/', '^', '\\', '<', '/', 'v', '\\'][sector]
}

fn clock(seconds: f64) -> String {
    let ms = (seconds * 1000.0).round() as u64;
    format!("{:02}:{:02}.{:03}", ms / 60_000, ms / 1000 % 60, ms % 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formatting() {
        assert_eq!(clock(42.3), "00:42.300");
        assert_eq!(clock(61.0), "01:01.000");
        assert_eq!(arrow(0.0), '>');
        assert_eq!(arrow(std::f64::consts::FRAC_PI_2), '^');
        assert_eq!(arrow(-std::f64::consts::FRAC_PI_2), 'v');
        assert_eq!(arrow(std::f64::consts::PI), '<');
    }
}
