use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
    Frame,
};

use crate::app::{format_bytes, App, SortMode};

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();

    let chunks = Layout::vertical([
        Constraint::Length(2), // header
        Constraint::Min(5),   // table
        Constraint::Length(2), // footer
    ])
    .split(area);

    draw_header(frame, app, chunks[0]);
    draw_table(frame, app, chunks[1]);
    draw_footer(frame, app, chunks[2]);
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let tailnet = if app.tailnet_name.is_empty() {
        "loading...".to_string()
    } else {
        app.tailnet_name.clone()
    };

    let status = if app.loading {
        " Loading...".to_string()
    } else if let Some(ref err) = app.error {
        format!(" Error: {err}")
    } else {
        format!(
            " Tailnet: {}    Nodes: {}/{} online    [{}s]",
            tailnet, app.online_nodes, app.total_nodes, app.refresh_interval_secs
        )
    };

    let header = Paragraph::new(Line::from(vec![
        Span::styled(" tailscale-top", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw(status),
    ]))
    .block(Block::default().borders(Borders::BOTTOM).border_style(Style::default().fg(Color::DarkGray)));

    frame.render_widget(header, area);
}

fn draw_table(frame: &mut Frame, app: &App, area: Rect) {
    let header_cells = ["Name", "IP", "OS", "TX", "RX", "St"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)));
    let header = Row::new(header_cells).height(1);

    let rows = app.nodes.iter().map(|node| {
        let status_symbol = if node.online { "●" } else { "○" };
        let status_color = if node.online { Color::Green } else { Color::DarkGray };

        let tx_display = match (node.online, node.tx_bytes) {
            (true, Some(b)) => format_bytes(b),
            (true, None) => "no wc".to_string(), // online but no webclient metrics
            (false, _) => "-".to_string(),
        };
        let rx_display = match (node.online, node.rx_bytes) {
            (true, Some(b)) => format_bytes(b),
            (true, None) => "no wc".to_string(),
            (false, _) => "-".to_string(),
        };

        let name_color = if node.online { Color::White } else { Color::DarkGray };

        Row::new(vec![
            Cell::from(node.name.clone()).style(Style::default().fg(name_color)),
            Cell::from(node.ip.clone()).style(Style::default().fg(Color::Blue)),
            Cell::from(node.os.clone()),
            Cell::from(tx_display).style(Style::default().fg(Color::Magenta)),
            Cell::from(rx_display).style(Style::default().fg(Color::Cyan)),
            Cell::from(status_symbol).style(Style::default().fg(status_color)),
        ])
    });

    let widths = [
        Constraint::Min(16),
        Constraint::Min(17),
        Constraint::Min(9),
        Constraint::Min(10),
        Constraint::Min(10),
        Constraint::Length(3),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .block(Block::default());

    frame.render_widget(table, area);
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let sort_indicator = |mode: SortMode, label: &str, key: &str| -> Vec<Span> {
        let active = app.sort_mode == mode;
        let style = if active {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        vec![
            Span::styled(format!("[{key}]"), style),
            Span::styled(
                format!("{label} "),
                if active {
                    Style::default().fg(Color::White)
                } else {
                    Style::default().fg(Color::DarkGray)
                },
            ),
        ]
    };

    let mut spans = vec![Span::raw(" Sort: ")];
    spans.extend(sort_indicator(SortMode::Name, "Name", "1"));
    spans.extend(sort_indicator(SortMode::TxDesc, "TX↓", "2"));
    spans.extend(sort_indicator(SortMode::RxDesc, "RX↓", "3"));
    spans.push(Span::raw("  │  "));
    spans.push(Span::styled("[q]", Style::default().fg(Color::Red)));
    spans.push(Span::raw("Quit "));
    spans.push(Span::styled("[r]", Style::default().fg(Color::Green)));
    spans.push(Span::raw("Refresh"));

    let footer = Paragraph::new(Line::from(spans))
        .block(Block::default().borders(Borders::TOP).border_style(Style::default().fg(Color::DarkGray)));

    frame.render_widget(footer, area);
}
