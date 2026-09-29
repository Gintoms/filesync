use crate::{
    agent,
    config::{self, AgentConfig, Mapping},
    model::Pending,
};
use anyhow::{Result, ensure};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{
    layout::{Constraint, Layout},
    style::{Color, Style},
    widgets::{Block, Paragraph, Row, Table, TableState},
};
use std::{
    io::IsTerminal,
    path::PathBuf,
    time::{Duration, Instant},
};

pub async fn run(config: &AgentConfig) -> Result<()> {
    ensure!(
        std::io::stdout().is_terminal(),
        "use `filesync status` or `filesync pending` for JSON output"
    );
    let url = agent::control_url(config)?;
    let token = config::node_secret(config)?;
    let client = agent::client(None)?;
    let mut terminal = ratatui::init();
    let result=async {
        let mut items:Vec<Pending>=Vec::new();
        let mut state=TableState::default();
        let mut status=String::from("Connecting to local agent...");
        let mut refreshed=Instant::now()-Duration::from_secs(10);
        let mut input:Option<(Mapping,String)>=None;
        loop {
            if input.is_none() && refreshed.elapsed()>=Duration::from_secs(2) {
                let selected=state.selected().and_then(|i|items.get(i)).map(|r|(r.upstream.clone(),r.path.clone()));
                match client.get(format!("{url}/pending")).bearer_auth(&token).send().await {
                    Ok(response) if response.status().is_success()=> {
                        items=response.json().await?;
                        let index=selected.and_then(|key|items.iter().position(|r|r.upstream==key.0 && r.path==key.1)).unwrap_or(0);
                        state.select(if items.is_empty(){None}else{Some(index.min(items.len()-1))});
                        status=format!("{} pending / conflict / error items (first 100); background sync runs separately",items.len());
                    }
                    _=>status="Agent unavailable; retrying...".into(),
                }
                refreshed=Instant::now();
            }
            terminal.draw(|frame| {
                let areas=Layout::vertical([Constraint::Length(3),Constraint::Min(3),Constraint::Length(4)]).split(frame.area());
                frame.render_widget(Paragraph::new(status.as_str()).block(Block::bordered().title("filesync · development preview")),areas[0]);
                let rows=items.iter().map(|item|Row::new(vec![item.upstream.chars().take(8).collect::<String>(),item.path.clone(),item.state.clone()]));
                let table=Table::new(rows,[Constraint::Length(10),Constraint::Min(20),Constraint::Length(12)])
                    .header(Row::new(["Upstream","File","State"]))
                    .block(Block::bordered().title("Unclaimed files are not downloaded"))
                    .row_highlight_style(Style::default().fg(Color::Yellow)).highlight_symbol("▸ ");
                frame.render_stateful_widget(table,areas[1],&mut state);
                let help=if let Some((mapping,value))=&input {format!("{} → {value}\nEnter: save mapping + sync   Esc: cancel\nEnter an absolute {} path",mapping.from,if mapping.from.ends_with('/') {"directory"}else{"file"})}
                    else {"↑/↓ select   m claim file   M claim parent prefix   r refresh   q close UI\nUpdates back up the old file; the last 3 backups are retained.".into()};
                frame.render_widget(Paragraph::new(help).block(Block::bordered()),areas[2]);
            })?;
            if !event::poll(Duration::from_millis(50))? {continue;}
            let Event::Key(key)=event::read()? else {continue;};
            if key.kind!=KeyEventKind::Press {continue;}
            if let Some((mapping,text))=&mut input {
                match key.code {
                    KeyCode::Esc=>input=None,
                    KeyCode::Backspace=>{text.pop();},
                    KeyCode::Char(ch)=>text.push(ch),
                    KeyCode::Enter=>{
                        mapping.to=PathBuf::from(text.as_str());
                        match client.post(format!("{url}/claim")).bearer_auth(&token).json(mapping).send().await?.error_for_status() {
                            Ok(_)=>status="Mapping saved; synchronization requested".into(),
                            Err(error)=>status=format!("Cannot claim: {error}"),
                        }
                        input=None;
                        refreshed=Instant::now();
                    },
                    _=>{},
                }
                continue;
            }
            match key.code {
                KeyCode::Char('q')=>break,
                KeyCode::Char('r')=>refreshed=Instant::now()-Duration::from_secs(10),
                KeyCode::Down|KeyCode::Char('j') if !items.is_empty()=>state.select(Some((state.selected().unwrap_or(0)+1).min(items.len()-1))),
                KeyCode::Up|KeyCode::Char('k')=>state.select(state.selected().map(|n|n.saturating_sub(1))),
                KeyCode::Char(ch @ ('m'|'M'))=>if let Some(item)=state.selected().and_then(|i|items.get(i)) {
                    let from=if ch=='M' {item.path.rsplit_once('/').map(|(parent,_)|format!("{parent}/")).unwrap_or_else(||item.path.clone())}else{item.path.clone()};
                    input=Some((Mapping{upstream:item.upstream.clone(),from,to:PathBuf::new()},String::new()));
                },
                _=>{},
            }
        }
        Ok(())
    }.await;
    ratatui::restore();
    result
}
