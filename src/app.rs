//! The messenger-style user interface.

use std::collections::HashSet;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use chrono::{Datelike, Local, TimeZone};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Frame, Id, Key, Layout, Margin, RichText, Sense, TextEdit,
    TextFormat, Vec2, text::LayoutJob, text::TextWrapping,
};

use crate::autoconfig::{self, Detected};
use crate::mail;
use crate::secret;
use crate::store::{Account, Conversation, Message, Security, Server, Store};
use crate::text;

enum Event {
    Detected(Result<Detected, String>),
    Saved(Result<(Account, String), String>),
    Refreshed(Result<usize, String>, bool),
    Sent(Result<(), String>),
    Read(Result<(), String>),
}

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Account,
    Signature,
}

struct SettingsForm {
    open: bool,
    tab: Tab,
    email: String,
    name: String,
    password: String,
    imap: Server,
    smtp: Server,
    signature: String,
    status: String,
    busy: bool,
}

struct Toast {
    text: String,
    error: bool,
    until: Instant,
}

pub struct MailerApp {
    store: Store,
    account: Option<Account>,
    password: Option<String>,
    signature: String,
    conversations: Vec<Conversation>,
    selected: Option<String>,
    thread: Vec<Message>,
    /// Message ids that show their quoted history.
    expanded: HashSet<String>,
    search: String,
    draft: String,
    subject: String,
    refreshing: bool,
    sending: bool,
    settings: SettingsForm,
    new_open: bool,
    new_to: String,
    toast: Option<Toast>,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    ctx: egui::Context,
}

struct Palette {
    chat_bg: Color32,
    mine: Color32,
    theirs: Color32,
    muted: Color32,
    accent: Color32,
    pill: Color32,
}

fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            chat_bg: Color32::from_rgb(11, 20, 26),
            mine: Color32::from_rgb(0, 92, 75),
            theirs: Color32::from_rgb(32, 44, 51),
            muted: Color32::from_rgb(134, 150, 160),
            accent: Color32::from_rgb(83, 166, 253),
            pill: Color32::from_rgb(31, 44, 51),
        }
    } else {
        Palette {
            chat_bg: Color32::from_rgb(239, 234, 226),
            mine: Color32::from_rgb(217, 253, 211),
            theirs: Color32::WHITE,
            muted: Color32::from_rgb(102, 119, 129),
            accent: Color32::from_rgb(43, 125, 233),
            pill: Color32::WHITE,
        }
    }
}

fn initials(name: &str) -> String {
    let base = name.split('@').next().unwrap_or(name);
    let mut parts = base
        .split(|c: char| c.is_whitespace() || ".-_".contains(c))
        .filter(|p| !p.is_empty());
    let a = parts.next().and_then(|p| p.chars().next()).unwrap_or('?');
    let b = parts.next().and_then(|p| p.chars().next());
    a.to_uppercase()
        .chain(b.into_iter().flat_map(char::to_uppercase))
        .collect()
}

fn avatar_color(key: &str) -> Color32 {
    let h = key.bytes().fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32)) % 360;
    egui::ecolor::Hsva::new(h as f32 / 360.0, 0.55, 0.55, 1.0).into()
}

fn paint_avatar(ui: &egui::Ui, center: egui::Pos2, radius: f32, name: &str, key: &str) {
    let p = ui.painter();
    p.circle_filled(center, radius, avatar_color(key));
    p.text(
        center,
        Align2::CENTER_CENTER,
        initials(name),
        FontId::proportional(radius * 0.8),
        Color32::WHITE,
    );
}

fn local(ts: i64) -> chrono::DateTime<Local> {
    Local.timestamp_opt(ts, 0).single().unwrap_or_else(Local::now)
}

fn short_time(ts: i64) -> String {
    let d = local(ts);
    let now = Local::now();
    if d.date_naive() == now.date_naive() {
        d.format("%H:%M").to_string()
    } else if (now - d).num_days() < 6 {
        d.format("%a").to_string()
    } else if d.year() == now.year() {
        d.format("%b %-d").to_string()
    } else {
        d.format("%Y-%m-%d").to_string()
    }
}

fn truncated(ui: &egui::Ui, text: &str, font: FontId, color: Color32, width: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = LayoutJob::single_section(
        text.to_string(),
        TextFormat {
            font_id: font,
            color,
            ..Default::default()
        },
    );
    job.wrap = TextWrapping::truncate_at_width(width.max(10.0));
    ui.painter().layout_job(job)
}

fn centered_pill(ui: &mut egui::Ui, label: &str, pal: &Palette) {
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_string(), FontId::proportional(12.0), pal.muted);
    let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), galley.size().y + 6.0), Sense::hover());
    let rect = egui::Rect::from_center_size(row.center(), galley.size() + Vec2::new(20.0, 6.0));
    ui.painter().rect_filled(rect, 8.0, pal.pill);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, pal.muted);
}

fn open_html(id: &str, html: &str, allow_remote: bool) -> Result<(), String> {
    let name: String = id.chars().filter(|c| c.is_ascii_alphanumeric()).take(40).collect();
    let path = std::env::temp_dir().join(format!("mailer-{name}.html"));
    std::fs::write(&path, text::html_document(html, allow_remote)).map_err(|e| e.to_string())?;
    open::that(&path).map_err(|e| e.to_string())
}

impl MailerApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let store = Store::open().expect("cannot open the database");
        let account = store.account();
        let password = account.as_ref().and_then(|a| secret::load(&a.email));
        let signature = store.signature();
        let (tx, rx) = channel();
        let mut app = MailerApp {
            store,
            account,
            password,
            signature,
            conversations: Vec::new(),
            selected: None,
            thread: Vec::new(),
            expanded: HashSet::new(),
            search: String::new(),
            draft: String::new(),
            subject: String::new(),
            refreshing: false,
            sending: false,
            settings: SettingsForm {
                open: false,
                tab: Tab::Account,
                email: String::new(),
                name: String::new(),
                password: String::new(),
                imap: Server {
                    port: 993,
                    ..Default::default()
                },
                smtp: Server {
                    port: 465,
                    ..Default::default()
                },
                signature: String::new(),
                status: String::new(),
                busy: false,
            },
            new_open: false,
            new_to: String::new(),
            toast: None,
            tx,
            rx,
            ctx: cc.egui_ctx.clone(),
        };
        app.reload();
        match (&app.account, &app.password) {
            (Some(_), Some(_)) => app.refresh(true),
            (Some(_), None) => {
                app.open_settings();
                app.settings.status = "Enter your password to connect.".into();
            }
            (None, _) => app.open_settings(),
        }
        app
    }

    fn spawn(&self, job: impl FnOnce() -> Event + Send + 'static) {
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(job());
            ctx.request_repaint();
        });
    }

    fn toast(&mut self, text: impl Into<String>, error: bool) {
        let secs = if error { 6 } else { 3 };
        self.toast = Some(Toast {
            text: text.into(),
            error,
            until: Instant::now() + Duration::from_secs(secs),
        });
    }

    fn reload(&mut self) {
        match self.store.conversations() {
            Ok(c) => self.conversations = c,
            Err(e) => self.toast(format!("Database error: {e}"), true),
        }
        if let Some(contact) = self.selected.clone() {
            self.thread = self.store.thread(&contact).unwrap_or_default();
        }
    }

    fn credentials(&self) -> Option<(Account, String)> {
        Some((self.account.clone()?, self.password.clone()?))
    }

    fn refresh(&mut self, silent: bool) {
        if self.refreshing {
            return;
        }
        let Some((account, password)) = self.credentials() else {
            if !silent {
                self.open_settings();
            }
            return;
        };
        self.refreshing = true;
        self.spawn(move || Event::Refreshed(mail::refresh(&account, &password), silent));
    }

    fn select(&mut self, contact: String) {
        self.selected = Some(contact.clone());
        self.thread = self.store.thread(&contact).unwrap_or_default();
        self.subject.clear();
        let unread = self.conversations.iter().any(|c| c.contact == contact && c.unread > 0);
        if let (true, Some((account, password))) = (unread, self.credentials()) {
            self.spawn(move || Event::Read(mail::mark_read(&account, &password, &contact)));
        }
    }

    fn send(&mut self) {
        let body = self.draft.trim().to_string();
        let (Some(to), false, false) = (self.selected.clone(), body.is_empty(), self.sending) else {
            return;
        };
        let Some((account, password)) = self.credentials() else {
            self.open_settings();
            return;
        };
        self.sending = true;
        let subject = self.subject.clone();
        let signature = self.signature.clone();
        self.spawn(move || Event::Sent(mail::send(&account, &password, &to, &subject, &body, &signature)));
    }

    fn open_settings(&mut self) {
        let s = &mut self.settings;
        if let Some(a) = &self.account {
            s.email = a.email.clone();
            s.name = a.name.clone();
            s.imap = a.imap.clone();
            s.smtp = a.smtp.clone();
        }
        s.password.clear();
        s.signature = self.signature.clone();
        s.status.clear();
        s.open = true;
    }

    fn handle_events(&mut self) {
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Event::Refreshed(result, silent) => {
                    self.refreshing = false;
                    match result {
                        Ok(n) => {
                            self.reload();
                            if !silent {
                                let msg = match n {
                                    0 => "No new mail".to_string(),
                                    1 => "1 new message".to_string(),
                                    n => format!("{n} new messages"),
                                };
                                self.toast(msg, false);
                            }
                        }
                        Err(e) => self.toast(format!("Refresh failed: {e}"), true),
                    }
                    // Opening a conversation during refresh may have missed unread mail.
                    if let Some(c) = self.selected.clone()
                        && self.conversations.iter().any(|x| x.contact == c && x.unread > 0)
                    {
                        self.select(c);
                    }
                }
                Event::Sent(result) => {
                    self.sending = false;
                    match result {
                        Ok(()) => {
                            self.draft.clear();
                            self.subject.clear();
                            self.reload();
                        }
                        Err(e) => self.toast(e, true),
                    }
                }
                Event::Read(result) => match result {
                    Ok(()) => self.reload(),
                    Err(e) => self.toast(format!("Could not mark as read: {e}"), true),
                },
                Event::Detected(result) => {
                    self.settings.busy = false;
                    match result {
                        Ok(d) => {
                            self.settings.imap = d.imap;
                            let mut status = format!("Found using {}.", d.source);
                            match d.smtp {
                                Some(s) => self.settings.smtp = s,
                                None => status.push_str(" No SMTP server found, please fill it in."),
                            }
                            self.settings.status = status;
                        }
                        Err(e) => self.settings.status = e,
                    }
                }
                Event::Saved(result) => {
                    self.settings.busy = false;
                    match result {
                        Ok((account, password)) => self.apply_account(account, password),
                        Err(e) => self.settings.status = e,
                    }
                }
            }
        }
    }

    fn apply_account(&mut self, account: Account, password: String) {
        let changed = self
            .account
            .as_ref()
            .is_none_or(|old| old.email != account.email || old.imap.host != account.imap.host);
        if changed {
            if let Some(old) = &self.account
                && old.email != account.email
            {
                secret::delete(&old.email);
            }
            let _ = self.store.reset_mail();
            self.selected = None;
            self.thread.clear();
        }
        if let Err(e) = self.store.save_account(&account) {
            self.settings.status = format!("Could not save settings: {e}");
            return;
        }
        let stored = secret::save(&account.email, &password);
        self.account = Some(account);
        self.password = Some(password);
        self.settings.open = false;
        match stored {
            Ok(()) => self.toast("Settings saved", false),
            Err(e) => self.toast(
                format!("Settings saved. The system keychain is not available ({e}), so the password is only kept until you close the app."),
                true,
            ),
        }
        self.reload();
        self.refresh(true);
    }

    fn save_settings(&mut self) {
        let s = &mut self.settings;
        if let Err(e) = self.store.save_signature(&s.signature) {
            s.status = format!("Could not save signature: {e}");
            return;
        }
        self.signature = s.signature.clone();

        let email = s.email.trim().to_string();
        if email.is_empty() {
            s.open = false;
            return;
        }
        let mut imap = s.imap.clone();
        let mut smtp = s.smtp.clone();
        imap.host = imap.host.trim().to_string();
        smtp.host = smtp.host.trim().to_string();
        if imap.username.trim().is_empty() {
            imap.username = email.clone();
        }
        if smtp.username.trim().is_empty() {
            smtp.username = imap.username.clone();
        }
        let account = Account {
            email,
            name: s.name.trim().to_string(),
            imap,
            smtp,
        };
        let password = if s.password.is_empty() {
            self.password.clone()
        } else {
            Some(s.password.clone())
        };
        let Some(password) = password else {
            s.status = "Enter your password.".into();
            return;
        };
        if account.imap.host.is_empty() {
            s.status = "Enter the IMAP server or use Detect settings.".into();
            return;
        }
        s.busy = true;
        s.status = "Testing connection...".into();
        self.spawn(move || Event::Saved(mail::test_account(&account, &password).map(|()| (account, password))));
    }

    /* ---------- drawing ---------- */

    fn sidebar(&mut self, ui: &mut egui::Ui, pal: &Palette) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let me = self
                .account
                .as_ref()
                .map(|a| {
                    if a.name.is_empty() {
                        a.email.clone()
                    } else {
                        a.name.clone()
                    }
                })
                .unwrap_or_else(|| "Not configured".into());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Settings").on_hover_text("Account and signature").clicked() {
                    self.open_settings();
                }
                if ui.button("New").on_hover_text("New conversation").clicked() {
                    self.new_to.clear();
                    self.new_open = true;
                }
                if self.refreshing {
                    ui.spinner();
                } else if ui.button("Refresh").on_hover_text("Pull new mail").clicked() {
                    self.refresh(false);
                }
                ui.add(egui::Label::new(RichText::new(me).strong().size(15.0)).truncate());
            });
        });
        ui.add_space(6.0);
        ui.add(
            TextEdit::singleline(&mut self.search)
                .hint_text("Search")
                .desired_width(f32::INFINITY),
        );
        ui.add_space(4.0);

        let q = self.search.to_lowercase();
        let list: Vec<Conversation> = self
            .conversations
            .iter()
            .filter(|c| {
                q.is_empty()
                    || format!("{} {} {}", c.name, c.contact, c.snippet)
                        .to_lowercase()
                        .contains(&q)
            })
            .cloned()
            .collect();

        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            if list.is_empty() {
                ui.add_space(20.0);
                ui.vertical_centered(|ui| {
                    let msg = if self.account.is_some() {
                        "No conversations yet"
                    } else {
                        "Open Settings to add your account"
                    };
                    ui.label(RichText::new(msg).color(pal.muted));
                });
            }
            for c in list {
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 64.0), Sense::click());
                let active = self.selected.as_deref() == Some(&c.contact);
                if active {
                    ui.painter()
                        .rect_filled(rect, 6.0, ui.visuals().selection.bg_fill.gamma_multiply(0.35));
                } else if resp.hovered() {
                    ui.painter()
                        .rect_filled(rect, 6.0, ui.visuals().widgets.hovered.weak_bg_fill);
                }
                let r = 22.0;
                paint_avatar(
                    ui,
                    egui::pos2(rect.left() + 10.0 + r, rect.center().y),
                    r,
                    &c.name,
                    &c.contact,
                );

                let x = rect.left() + 20.0 + 2.0 * r;
                let right = rect.right() - 8.0;
                let text_color = ui.visuals().text_color();
                let strong = ui.visuals().strong_text_color();
                let time = truncated(
                    ui,
                    &short_time(c.date),
                    FontId::proportional(12.0),
                    if c.unread > 0 { pal.accent } else { pal.muted },
                    100.0,
                );
                let time_w = time.size().x;
                ui.painter()
                    .galley(egui::pos2(right - time_w, rect.top() + 13.0), time, text_color);

                let name = truncated(
                    ui,
                    &c.name,
                    FontId::proportional(15.0),
                    if c.unread > 0 { strong } else { text_color },
                    right - x - time_w - 8.0,
                );
                ui.painter().galley(egui::pos2(x, rect.top() + 11.0), name, text_color);

                let mut snippet_w = right - x;
                if c.unread > 0 {
                    let badge = truncated(
                        ui,
                        &c.unread.to_string(),
                        FontId::proportional(11.0),
                        Color32::WHITE,
                        60.0,
                    );
                    let w = (badge.size().x + 12.0).max(20.0);
                    let b = egui::Rect::from_min_size(egui::pos2(right - w, rect.top() + 34.0), Vec2::new(w, 20.0));
                    ui.painter().rect_filled(b, 10.0, pal.accent);
                    let size = badge.size();
                    ui.painter().galley(b.center() - size / 2.0, badge, Color32::WHITE);
                    snippet_w -= w + 8.0;
                }
                let snippet = truncated(ui, &c.snippet, FontId::proportional(13.0), pal.muted, snippet_w);
                ui.painter()
                    .galley(egui::pos2(x, rect.top() + 35.0), snippet, pal.muted);

                if resp.clicked() {
                    self.select(c.contact.clone());
                }
            }
        });
    }

    fn chat(&mut self, ui: &mut egui::Ui, pal: &Palette) {
        let Some(contact) = self.selected.clone() else {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("Select a conversation").color(pal.muted).size(16.0));
            });
            return;
        };
        let name = self
            .conversations
            .iter()
            .find(|c| c.contact == contact)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| contact.clone());

        egui::Panel::top("chat_head")
            .frame(Frame::side_top_panel(ui.style()).inner_margin(Margin::symmetric(12, 8)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::hover());
                    paint_avatar(ui, rect.center(), 20.0, &name, &contact);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&name).strong().size(15.0));
                        ui.label(RichText::new(&contact).color(pal.muted).size(12.0));
                    });
                });
            });

        egui::Panel::bottom("composer")
            .frame(Frame::side_top_panel(ui.style()).inner_margin(Margin::symmetric(12, 8)))
            .show(ui, |ui| self.composer(ui, pal));

        egui::CentralPanel::no_frame().show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    ui.add_space(10.0);
                    self.messages(ui, pal);
                    ui.add_space(10.0);
                });
        });
    }

    fn messages(&mut self, ui: &mut egui::Ui, pal: &Palette) {
        if self.thread.is_empty() {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| ui.label(RichText::new("No messages yet. Say hello!").color(pal.muted)));
            return;
        }
        let thread = self.thread.clone();
        let mut last_day = None;
        let mut last_topic = None;
        for m in &thread {
            let day = local(m.date).date_naive();
            if last_day != Some(day) {
                last_day = Some(day);
                ui.add_space(8.0);
                centered_pill(ui, &local(m.date).format("%A, %B %-d, %Y").to_string(), pal);
                ui.add_space(4.0);
            }
            let topic = text::base_subject(&m.subject);
            if last_topic.as_ref() != Some(&topic) {
                last_topic = Some(topic);
                ui.vertical_centered(|ui| {
                    let s = if m.subject.is_empty() {
                        "(no subject)"
                    } else {
                        &m.subject
                    };
                    ui.label(RichText::new(s).italics().size(12.0).color(pal.muted));
                });
            }
            self.bubble(ui, m, pal);
            ui.add_space(4.0);
        }
    }

    /// One chat bubble. Laid out by hand so its width follows the text.
    fn bubble(&mut self, ui: &mut egui::Ui, m: &Message, pal: &Palette) {
        const SIDE: f32 = 16.0;
        const PAD: Vec2 = Vec2::new(10.0, 6.0);
        let max_w = ((ui.available_width() - 2.0 * SIDE) * 0.75).max(160.0);

        let expanded = self.expanded.contains(&m.message_id);
        let has_more = !m.short.is_empty() && m.body.trim() != m.short.trim();
        let body = if expanded || m.short.is_empty() {
            m.body.trim()
        } else {
            m.short.as_str()
        };
        let body = if body.is_empty() { "(empty message)" } else { body };
        let text_color = if ui.visuals().dark_mode {
            Color32::from_rgb(233, 237, 239)
        } else {
            Color32::from_rgb(17, 27, 33)
        };

        let inner_w = max_w - 2.0 * PAD.x;
        let wrapped = |s: &str, size: f32, color: Color32| {
            ui.painter()
                .layout(s.to_string(), FontId::proportional(size), color, inner_w)
        };
        let body_galley = wrapped(body, 14.0, text_color);
        let attachments = (!m.attachments.is_empty())
            .then(|| wrapped(&format!("Attachments: {}", m.attachments.join(", ")), 12.0, pal.muted));

        let mut links: Vec<&str> = Vec::new();
        if has_more {
            links.push(if expanded { "hide quoted" } else { "show quoted" });
        }
        if m.html.is_some() {
            links.extend(["view HTML", "with images"]);
        }
        let time = local(m.date).format("%H:%M").to_string();
        let spacing = ui.spacing().item_spacing.x;
        let text_w = |s: &str| {
            ui.painter()
                .layout_no_wrap(s.to_string(), FontId::proportional(11.0), pal.muted)
                .size()
                .x
        };
        let link_pad = 2.0 * ui.spacing().button_padding.x;
        let meta_w = links.iter().map(|l| text_w(l) + link_pad + spacing).sum::<f32>() + text_w(&time);
        let meta_h = ui.spacing().interact_size.y;

        let content_w = body_galley
            .size()
            .x
            .max(attachments.as_ref().map_or(0.0, |a| a.size().x))
            .max(meta_w)
            .min(inner_w);
        let content_h = body_galley.size().y + attachments.as_ref().map_or(0.0, |a| a.size().y + 4.0) + meta_h;
        let size = Vec2::new(content_w, content_h) + 2.0 * PAD;

        let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), size.y), Sense::hover());
        let x = if m.outgoing {
            row.right() - SIDE - size.x
        } else {
            row.left() + SIDE
        };
        let rect = egui::Rect::from_min_size(egui::pos2(x, row.top()), size);
        let radius = if m.outgoing {
            CornerRadius {
                nw: 8,
                ne: 0,
                sw: 8,
                se: 8,
            }
        } else {
            CornerRadius {
                nw: 0,
                ne: 8,
                sw: 8,
                se: 8,
            }
        };
        ui.painter()
            .rect_filled(rect, radius, if m.outgoing { pal.mine } else { pal.theirs });

        let mut inner = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect.shrink2(PAD))
                .layout(Layout::top_down(Align::Min)),
        );
        let ui = &mut inner;
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.add(egui::Label::new(body_galley).selectable(true));
        if let Some(a) = attachments {
            ui.add_space(4.0);
            ui.add(egui::Label::new(a));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(time).size(11.0).color(pal.muted));
            for label in links.iter().rev() {
                let b = egui::Button::new(RichText::new(*label).size(11.0).color(pal.accent)).frame(false);
                if !ui.add(b).clicked() {
                    continue;
                }
                match *label {
                    "view HTML" | "with images" => {
                        let html = m.html.as_deref().unwrap_or_default();
                        if let Err(e) = open_html(&m.message_id, html, *label == "with images") {
                            self.toast(format!("Could not open browser: {e}"), true);
                        }
                    }
                    _ if expanded => {
                        self.expanded.remove(&m.message_id);
                    }
                    _ => {
                        self.expanded.insert(m.message_id.clone());
                    }
                }
            }
        });
    }

    fn composer(&mut self, ui: &mut egui::Ui, pal: &Palette) {
        ui.add(
            TextEdit::singleline(&mut self.subject)
                .hint_text("Subject (leave empty to reply to the last message)")
                .frame(Frame::NONE)
                .text_color(pal.muted)
                .desired_width(f32::INFINITY),
        );
        ui.horizontal(|ui| {
            let send_w = 70.0;
            let edit = ui.add_sized(
                Vec2::new(ui.available_width() - send_w - 8.0, 44.0),
                TextEdit::multiline(&mut self.draft)
                    .hint_text("Write a message (Ctrl+Enter to send)")
                    .desired_rows(2),
            );
            let ctrl_enter = edit.has_focus() && ui.input(|i| i.key_pressed(Key::Enter) && i.modifiers.command);
            let button = ui.add_enabled(
                !self.sending && !self.draft.trim().is_empty(),
                egui::Button::new(if self.sending { "Sending" } else { "Send" }).min_size(Vec2::new(send_w, 44.0)),
            );
            if ctrl_enter || button.clicked() {
                self.send();
            }
        });
    }

    fn server_fields(ui: &mut egui::Ui, id: &str, server: &mut Server) {
        egui::Grid::new(id).num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            ui.label("Server");
            ui.add(TextEdit::singleline(&mut server.host).desired_width(260.0));
            ui.end_row();
            ui.label("Port");
            let mut port = server.port.to_string();
            if ui.add(TextEdit::singleline(&mut port).desired_width(80.0)).changed() {
                server.port = port.trim().parse().unwrap_or(0);
            }
            ui.end_row();
            ui.label("Security");
            egui::ComboBox::from_id_salt(format!("{id}_sec"))
                .selected_text(server.security.label())
                .show_ui(ui, |ui| {
                    for s in [Security::Tls, Security::StartTls, Security::None] {
                        ui.selectable_value(&mut server.security, s, s.label());
                    }
                });
            ui.end_row();
            ui.label("Username");
            ui.add(
                TextEdit::singleline(&mut server.username)
                    .hint_text("same as email")
                    .desired_width(260.0),
            );
            ui.end_row();
        });
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings.open {
            return;
        }
        let mut open = true;
        let mut detect = false;
        let mut save = false;
        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(460.0)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                let s = &mut self.settings;
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut s.tab, Tab::Account, "Account");
                    ui.selectable_value(&mut s.tab, Tab::Signature, "Signature");
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .max_height(ctx.content_rect().height() - 200.0)
                    .show(ui, |ui| match s.tab {
                        Tab::Account => {
                            egui::Grid::new("acct")
                                .num_columns(2)
                                .spacing([8.0, 6.0])
                                .show(ui, |ui| {
                                    ui.label("Email");
                                    ui.horizontal(|ui| {
                                        ui.add(TextEdit::singleline(&mut s.email).desired_width(220.0));
                                        detect =
                                            ui.add_enabled(!s.busy, egui::Button::new("Detect settings")).clicked();
                                    });
                                    ui.end_row();
                                    ui.label("Display name");
                                    ui.add(TextEdit::singleline(&mut s.name).desired_width(260.0));
                                    ui.end_row();
                                    ui.label("Password");
                                    let hint = if self.password.is_some() { "(unchanged)" } else { "" };
                                    ui.add(
                                        TextEdit::singleline(&mut s.password)
                                            .password(true)
                                            .hint_text(hint)
                                            .desired_width(260.0),
                                    );
                                    ui.end_row();
                                });
                            if !secret::available() {
                                ui.label(
                                    RichText::new(
                                        "No system keychain found. The password is only kept until you close the app.",
                                    )
                                    .size(12.0)
                                    .color(ui.visuals().warn_fg_color),
                                );
                            }
                            ui.add_space(8.0);
                            ui.strong("IMAP (incoming)");
                            Self::server_fields(ui, "imap", &mut s.imap);
                            ui.add_space(8.0);
                            ui.strong("SMTP (outgoing)");
                            Self::server_fields(ui, "smtp", &mut s.smtp);
                        }
                        Tab::Signature => {
                            ui.label("HTML signature, added to every message you send.");
                            ui.add(
                                TextEdit::multiline(&mut s.signature)
                                    .code_editor()
                                    .desired_rows(8)
                                    .desired_width(f32::INFINITY)
                                    .hint_text("<b>Jane Doe</b><br><a href=\"https://example.com\">example.com</a>"),
                            );
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Plain text version:").size(12.0));
                                if ui.button("Preview in browser").clicked() {
                                    let _ = open_html("signature-preview", &s.signature, true);
                                }
                            });
                            Frame::group(ui.style()).show(ui, |ui| {
                                ui.set_min_width(ui.available_width());
                                ui.label(text::html_to_text(&s.signature));
                            });
                        }
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    if s.busy {
                        ui.spinner();
                    }
                    ui.add(egui::Label::new(RichText::new(&s.status).size(12.0)).wrap());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        save = ui.add_enabled(!s.busy, egui::Button::new("Save")).clicked();
                    });
                });
                ui.label(
                    RichText::new(format!("mailer {}", env!("CARGO_PKG_VERSION")))
                        .size(11.0)
                        .weak(),
                );
            });
        if !open {
            self.settings.open = false;
        }
        if detect {
            let email = self.settings.email.clone();
            self.settings.busy = true;
            self.settings.status = "Looking up DNS...".into();
            self.spawn(move || Event::Detected(autoconfig::discover(&email)));
        }
        if save {
            self.save_settings();
        }
    }

    fn new_window(&mut self, ctx: &egui::Context) {
        if !self.new_open {
            return;
        }
        let mut open = true;
        let mut start = false;
        egui::Window::new("New conversation")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                let edit = ui.add(
                    TextEdit::singleline(&mut self.new_to)
                        .hint_text("someone@example.com")
                        .desired_width(280.0),
                );
                edit.request_focus();
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                start = ui.button("Start").clicked() || enter;
            });
        let to = self.new_to.trim().to_lowercase();
        if start && to.contains('@') {
            self.new_open = false;
            self.select(to);
        } else if !open {
            self.new_open = false;
        }
    }

    fn toast_overlay(&mut self, ctx: &egui::Context) {
        let Some(t) = &self.toast else { return };
        if Instant::now() > t.until {
            self.toast = None;
            return;
        }
        let fill = if t.error {
            Color32::from_rgb(198, 40, 40)
        } else {
            Color32::from_rgb(50, 50, 50)
        };
        egui::Area::new(Id::unique("toast"))
            .anchor(Align2::CENTER_BOTTOM, Vec2::new(0.0, -90.0))
            .order(egui::Order::Tooltip)
            .show(ctx, |ui| {
                Frame::new()
                    .fill(fill)
                    .corner_radius(8)
                    .inner_margin(Margin::symmetric(16, 10))
                    .show(ui, |ui| {
                        ui.set_max_width(480.0);
                        ui.label(RichText::new(&t.text).color(Color32::WHITE));
                    });
            });
        ctx.request_repaint_after(Duration::from_millis(250));
    }
}

impl eframe::App for MailerApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.handle_events();
        let ctx = ui.ctx().clone();
        let pal = palette(ui.visuals().dark_mode);

        egui::Panel::left("sidebar")
            .resizable(true)
            .default_size(330.0)
            .size_range(240.0..=520.0)
            .frame(Frame::side_top_panel(ui.style()).inner_margin(Margin::symmetric(10, 4)))
            .show(ui, |ui| self.sidebar(ui, &pal));

        egui::CentralPanel::default()
            .frame(Frame::new().fill(pal.chat_bg))
            .show(ui, |ui| self.chat(ui, &pal));

        self.settings_window(&ctx);
        self.new_window(&ctx);
        self.toast_overlay(&ctx);
    }
}
