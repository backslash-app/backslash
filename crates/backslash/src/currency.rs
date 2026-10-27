//! Exchange rates from the European Central Bank (via frankfurter.app, no API
//! key), cached for 12 hours in `~/.local/state/backslash/rates.json`.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use backslash_core::units::Rates;
use backslash_core::{paths, store};
use gtk::glib;

const URL: &str = "https://api.frankfurter.app/latest?base=EUR";
const MAX_AGE: u64 = 12 * 3600;

pub enum RatesStatus {
    Ready(Rates),
    Loading,
    Failed(String),
}

#[derive(Default)]
struct Inner {
    rates: Option<Rates>,
    loading: bool,
    error: Option<String>,
    /// Unix time of the last download attempt, to avoid hammering the
    /// service (and looping) while offline.
    last_attempt: u64,
}

const RETRY_AFTER: u64 = 60;

type Callback = Rc<RefCell<Option<Box<dyn Fn()>>>>;

/// Rates manager living on the GTK thread. Asking for rates starts a download
/// when needed; `on_update` runs when it finishes.
#[derive(Clone)]
pub struct Currency {
    inner: Rc<RefCell<Inner>>,
    on_update: Callback,
}

impl Currency {
    pub fn new() -> Self {
        let cached: Rates = store::read_json(&paths::state_dir().join("rates.json"));
        let rates = (!cached.rates.is_empty()).then_some(cached);
        Self { inner: Rc::new(RefCell::new(Inner { rates, ..Default::default() })), on_update: Rc::default() }
    }

    pub fn connect_update(&self, f: impl Fn() + 'static) {
        *self.on_update.borrow_mut() = Some(Box::new(f));
    }

    /// Current rates; stale or missing rates trigger a refresh. Stale rates are
    /// still returned meanwhile, so offline use keeps working.
    pub fn status(&self) -> RatesStatus {
        let fresh = self
            .inner
            .borrow()
            .rates
            .as_ref()
            .is_some_and(|r| crate::launcher::now().saturating_sub(r.fetched) < MAX_AGE);
        if !fresh {
            self.refresh();
        }
        let inner = self.inner.borrow();
        match (&inner.rates, &inner.error) {
            (Some(r), _) => RatesStatus::Ready(r.clone()),
            (None, Some(e)) if !inner.loading => RatesStatus::Failed(e.clone()),
            _ => RatesStatus::Loading,
        }
    }

    fn refresh(&self) {
        {
            let mut inner = self.inner.borrow_mut();
            let now = crate::launcher::now();
            if inner.loading || now.saturating_sub(inner.last_attempt) < RETRY_AFTER {
                return;
            }
            inner.loading = true;
            inner.last_attempt = now;
        }
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = tx.send_blocking(fetch());
        });
        let this = self.clone();
        glib::spawn_future_local(async move {
            let result = rx.recv().await.unwrap_or_else(|_| Err("download thread died".into()));
            {
                let mut inner = this.inner.borrow_mut();
                inner.loading = false;
                match result {
                    Ok(rates) => {
                        let _ = store::write_json(&paths::state_dir().join("rates.json"), &rates);
                        inner.rates = Some(rates);
                        inner.error = None;
                    }
                    Err(e) => {
                        eprintln!("backslash: exchange rates: {e}");
                        inner.error = Some(e);
                    }
                }
            }
            if let Some(f) = this.on_update.borrow().as_ref() {
                f();
            }
        });
    }
}

fn fetch() -> Result<Rates, String> {
    let agent = backslash_core::http::agent(Duration::from_secs(8));
    let mut response = agent.get(URL).call().map_err(|e| format!("cannot reach the rates service ({e})"))?;
    let mut rates: Rates = response.body_mut().read_json().map_err(|e| format!("unexpected answer ({e})"))?;
    rates.fetched = crate::launcher::now();
    Ok(rates)
}
