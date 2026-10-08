//! Account-scoped encrypted history. All Keychain, SQLite and serialization work stays off GPUI.
use crate::{
    Wake,
    model::{self, ChatSummary, Delivery, Message},
    settings,
};
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, sync::mpsc, thread, time::Duration};

pub(crate) struct CachedChat {
    pub summary: ChatSummary,
    pub draft: String,
    pub pending: Vec<Message>,
}

pub(crate) struct ExitSnapshot {
    pub prefs: settings::Settings,
    pub chats: Vec<CachedChat>,
    pub removed: Vec<(String, Vec<String>)>,
}

pub(crate) enum Command {
    Name(String),
    Save {
        summary: ChatSummary,
        messages: Vec<Message>,
        remove: Vec<String>,
    },
    Draft {
        chat_id: String,
        text: String,
    },
    History {
        chat_id: String,
        revision: u64,
    },
    Prepare {
        summary: ChatSummary,
        message: Box<Message>,
    },
    Delete {
        chat_id: String,
        message_id: String,
    },
    ClearHistory,
    SaveBeforeExit {
        snapshot: Box<ExitSnapshot>,
        reply: async_channel::Sender<Result<(), String>>,
    },
}

pub(crate) enum Event {
    Ready {
        name: String,
        chats: Vec<CachedChat>,
    },
    History {
        chat_id: String,
        revision: u64,
        messages: Vec<Message>,
    },
    Prepared {
        chat_id: String,
        message: Box<Message>,
        saved: bool,
    },
    Cleared,
    Error(String),
}

pub(crate) struct Store {
    commands: mpsc::SyncSender<Command>,
    pub events: mpsc::Receiver<Event>,
}

impl Store {
    pub fn new(
        account: String,
        name: String,
        legacy: BTreeMap<String, String>,
        wake: Wake,
    ) -> Self {
        let (commands, requests) = mpsc::sync_channel(128);
        let (events, responses) = mpsc::channel();
        let owner = account;
        thread::spawn(move || {
            let emit = |event| {
                let _ = events.send(event);
                let _ = wake.try_send(());
            };
            let mut db = match open(&owner).and_then(|mut db| {
                initialize(&mut db, &name, legacy)?;
                let (name, chats) = load(&db)?;
                emit(Event::Ready { name, chats });
                Ok(db)
            }) {
                Ok(db) => db,
                Err(_) => {
                    emit(Event::Error("Could not open encrypted history. Check Keychain access and free disk space. Existing data was not replaced.".into()));
                    return;
                }
            };
            let mut cache_writes = 0usize;
            while let Ok(command) = requests.recv() {
                let result: Result<()> = (|| {
                    match command {
                        Command::SaveBeforeExit { snapshot, reply } => {
                            let result = save_before_exit(&mut db, *snapshot).map_err(|_| "Could not save drafts before quitting. Check free disk space and keep the app open; your text is still here.".to_owned());
                            let _ = reply.try_send(result);
                        }
                        Command::Name(name) => {
                            db.execute(
                                "INSERT OR REPLACE INTO metadata VALUES ('name', ?1)",
                                [&name],
                            )?;
                        }
                        Command::Draft { chat_id, text } => {
                            ensure_chat(&db, &chat_id)?;
                            db.execute(
                                "UPDATE chats SET draft=?2 WHERE id=?1",
                                params![chat_id, text],
                            )?;
                        }
                        Command::Save {
                            summary,
                            messages,
                            remove,
                        } => {
                            let tx = db.transaction()?;
                            tx.execute("INSERT INTO chats(id,data) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET data=excluded.data",
                                params![summary.id, serde_json::to_string(&summary)?])?;
                            for message in messages {
                                save_message(&tx, &summary.id, &message)?;
                            }
                            for id in remove {
                                tx.execute(
                                    "DELETE FROM messages WHERE chat=?1 AND id=?2",
                                    params![summary.id, id],
                                )?;
                            }
                            prune(&tx, &summary.id)?;
                            cache_writes += 1;
                            if cache_writes.is_multiple_of(50) {
                                prune_global(&tx)?;
                            }
                            tx.commit()?;
                        }
                        Command::History { chat_id, revision } => {
                            let mut query = db.prepare("SELECT data FROM messages WHERE chat=?1 ORDER BY created DESC, id DESC LIMIT 500")?;
                            let rows =
                                query.query_map([&chat_id], |row| row.get::<_, String>(0))?;
                            let mut messages = Vec::new();
                            for row in rows {
                                messages.push(recovered(&row?)?);
                            }
                            emit(Event::History {
                                chat_id,
                                revision,
                                messages,
                            });
                        }
                        Command::Prepare { summary, message } => {
                            // Commit BEFORE allowing the HTTP worker to POST. No automatic replay.
                            let saved = (|| -> Result<()> {
                                let tx = db.transaction()?;
                                tx.execute("INSERT INTO chats(id,data) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET data=excluded.data",
                                    params![summary.id, serde_json::to_string(&summary)?])?;
                                save_message(&tx, &summary.id, &message)?;
                                tx.execute("UPDATE chats SET draft='' WHERE id=?1", [&summary.id])?;
                                tx.commit()?;
                                Ok(())
                            })().is_ok();
                            emit(Event::Prepared {
                                chat_id: summary.id,
                                message,
                                saved,
                            });
                        }
                        Command::Delete {
                            chat_id,
                            message_id,
                        } => {
                            let json: Option<String> = db
                                .query_row(
                                    "SELECT data FROM messages WHERE chat=?1 AND id=?2",
                                    params![chat_id, message_id],
                                    |r| r.get(0),
                                )
                                .optional()?;
                            if let Some(json) = json {
                                let mut message: Message = serde_json::from_str(&json)?;
                                message.delete();
                                save_message(&db, &chat_id, &message)?;
                            }
                        }
                        Command::ClearHistory => {
                            let tx = db.transaction()?;
                            tx.execute("DELETE FROM messages WHERE pending=0", [])?;
                            let summaries: Vec<String> = tx
                                .prepare("SELECT data FROM chats")?
                                .query_map([], |r| r.get(0))?
                                .collect::<rusqlite::Result<_>>()?;
                            for json in summaries {
                                let mut summary: ChatSummary = serde_json::from_str(&json)?;
                                summary.preview.clear();
                                summary.preview_id = None;
                                tx.execute(
                                    "UPDATE chats SET data=?2 WHERE id=?1",
                                    params![summary.id, serde_json::to_string(&summary)?],
                                )?;
                            }
                            tx.commit()?;
                            db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); VACUUM;")?;
                            emit(Event::Cleared);
                        }
                    }
                    Ok(())
                })();
                if result.is_err() {
                    emit(Event::Error("Could not update encrypted history. Check free disk space; keep the app open to preserve unsaved changes.".into()));
                }
            }
        });
        Self {
            commands,
            events: responses,
        }
    }

    pub fn send(&self, command: Command) -> Result<(), String> {
        self.commands.try_send(command).map_err(|_| {
            "Local storage is busy or unavailable. Keep the app open and try again.".into()
        })
    }
}

fn save_before_exit(db: &mut Connection, snapshot: ExitSnapshot) -> Result<()> {
    let tx = db.transaction()?;
    for (chat, ids) in snapshot.removed {
        for id in ids {
            tx.execute(
                "DELETE FROM messages WHERE chat=?1 AND id=?2",
                params![chat, id],
            )?;
        }
    }
    for chat in snapshot.chats {
        tx.execute("INSERT INTO chats(id,data,draft) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET data=excluded.data,draft=excluded.draft",
            params![chat.summary.id, serde_json::to_string(&chat.summary)?, chat.draft])?;
        for message in chat.pending {
            save_message(&tx, &chat.summary.id, &message)?;
        }
    }
    tx.commit()?;
    snapshot.prefs.save().map_err(anyhow::Error::msg)
}

fn open(account: &str) -> Result<Connection> {
    // Debug/release have different signing identities and Keychain namespaces.
    let hash = format!("{:x}", Sha256::digest(account.as_bytes()));
    let directory =
        settings::directory()
            .map_err(anyhow::Error::msg)?
            .join(if cfg!(debug_assertions) {
                "history-debug"
            } else {
                "history"
            });
    std::fs::create_dir_all(&directory)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
    }
    let path = directory.join(format!("{hash}.sqlite3"));
    // Serialize first-use key creation across processes as well as reconnecting workers.
    let mut lock_options = std::fs::OpenOptions::new();
    lock_options.write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        lock_options.mode(0o600);
    }
    let lock = lock_options.open(path.with_extension("lock"))?;
    lock.lock()?;
    let secret = format!("history:{hash}");
    let key = match settings::read_secret(&secret).map_err(anyhow::Error::msg)? {
        Some(key) => key,
        None => {
            if path.exists() {
                bail!("History key unavailable");
            }
            let mut bytes = [0u8; 32];
            getrandom::fill(&mut bytes)
                .map_err(|_| anyhow::anyhow!("Could not generate history key"))?;
            let key: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            settings::write_secret(&secret, &key).map_err(anyhow::Error::msg)?;
            key
        }
    };
    if key.len() != 64 || !key.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("Invalid history key");
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(&path)?;
    let db = Connection::open(&path)?;
    db.busy_timeout(Duration::from_secs(3))?;
    db.execute_batch(&format!("PRAGMA key = \"x'{key}'\";"))?;
    let cipher: String = db.query_row("PRAGMA cipher_version", [], |row| row.get(0))?;
    if cipher.is_empty() {
        bail!("Encryption unavailable");
    }
    let version: i64 = db.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version > 1 {
        bail!("Newer history format");
    }
    db.execute_batch("PRAGMA temp_store=MEMORY; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
        PRAGMA secure_delete=ON;
        CREATE TABLE IF NOT EXISTS metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS chats (id TEXT PRIMARY KEY, data TEXT NOT NULL, draft TEXT NOT NULL DEFAULT '');
        CREATE TABLE IF NOT EXISTS messages (chat TEXT NOT NULL, id TEXT NOT NULL, created INTEGER NOT NULL,
            pending INTEGER NOT NULL, data TEXT NOT NULL, PRIMARY KEY(chat,id));
        CREATE INDEX IF NOT EXISTS message_time ON messages(chat, created DESC);
        CREATE INDEX IF NOT EXISTS cache_time ON messages(pending, created DESC);
        PRAGMA user_version=1;")?;
    Ok(db)
}

fn initialize(db: &mut Connection, name: &str, legacy: BTreeMap<String, String>) -> Result<()> {
    let tx = db.transaction()?;
    if !name.is_empty() {
        tx.execute("INSERT OR REPLACE INTO metadata VALUES('name',?1)", [name])?;
    }
    if tx
        .query_row(
            "SELECT value FROM metadata WHERE key='drafts-imported'",
            [],
            |r| r.get::<_, String>(0),
        )
        .optional()?
        .is_none()
    {
        for (id, draft) in legacy {
            let summary = ChatSummary {
                id: id.clone(),
                ..Default::default()
            };
            tx.execute(
                "INSERT OR IGNORE INTO chats(id,data,draft) VALUES(?1,?2,?3)",
                params![id, serde_json::to_string(&summary)?, draft],
            )?;
        }
        tx.execute("INSERT INTO metadata VALUES('drafts-imported','1')", [])?;
    }
    tx.commit()?;
    Ok(())
}

fn load(db: &Connection) -> Result<(String, Vec<CachedChat>)> {
    let name = db
        .query_row("SELECT value FROM metadata WHERE key='name'", [], |r| {
            r.get(0)
        })
        .optional()?
        .unwrap_or_default();
    let mut query = db.prepare("SELECT id,data,draft FROM chats")?;
    let rows = query.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get(2)?))
    })?;
    let mut chats = Vec::new();
    for row in rows {
        let (id, json, draft) = row?;
        let mut pending = Vec::new();
        for json in db
            .prepare("SELECT data FROM messages WHERE chat=?1 AND pending=1")?
            .query_map([id], |r| r.get::<_, String>(0))?
        {
            pending.push(recovered(&json?)?);
        }
        chats.push(CachedChat {
            summary: serde_json::from_str(&json)?,
            draft,
            pending,
        });
    }
    Ok((name, chats))
}

fn recovered(json: &str) -> Result<Message> {
    let mut message: Message = serde_json::from_str(json).context("Invalid cached message")?;
    if message.delivery != Delivery::Sent {
        message.delivery = Delivery::Unconfirmed;
    }
    Ok(message)
}

fn ensure_chat(db: &Connection, id: &str) -> Result<()> {
    let summary = ChatSummary {
        id: id.into(),
        ..Default::default()
    };
    db.execute(
        "INSERT OR IGNORE INTO chats(id,data) VALUES(?1,?2)",
        params![id, serde_json::to_string(&summary)?],
    )?;
    Ok(())
}

fn save_message(db: &Connection, chat: &str, message: &Message) -> Result<()> {
    db.execute(
        "INSERT INTO messages VALUES(?1,?2,?3,?4,?5) ON CONFLICT(chat,id) DO UPDATE SET
        created=excluded.created,pending=excluded.pending,data=excluded.data",
        params![
            chat,
            message.id,
            model::parse_time(&message.created_at).map_or(0, |t| t.unix_timestamp()),
            message.delivery != Delivery::Sent,
            serde_json::to_string(message)?
        ],
    )?;
    Ok(())
}

fn prune(db: &Connection, chat: &str) -> Result<()> {
    // Pending sends and drafts are user data, not evictable cache.
    db.execute(
        "DELETE FROM messages WHERE chat=?1 AND pending=0 AND id NOT IN
        (SELECT id FROM messages WHERE chat=?1 AND pending=0 ORDER BY created DESC LIMIT 500)",
        [chat],
    )?;
    Ok(())
}

fn prune_global(db: &Connection) -> Result<()> {
    db.execute(
        "DELETE FROM messages WHERE pending=0 AND rowid NOT IN
        (SELECT rowid FROM messages WHERE pending=0 ORDER BY created DESC LIMIT 20000)",
        [],
    )?;
    db.execute("DELETE FROM messages WHERE rowid IN (SELECT rowid FROM
        (SELECT rowid, sum(length(data)) OVER (ORDER BY created DESC, rowid DESC) AS bytes FROM messages WHERE pending=0)
        WHERE bytes > 67108864)", [])?;
    Ok(())
}
