//! Sparkle owns download, signature verification, bundle replacement and native update UI.
//! Keep the controller and its one-shot relaunch continuation on the AppKit main thread.
pub(crate) struct Updates {
    #[cfg(feature = "auto-update")]
    updater: Option<sparkle_updater::SparkleUpdater>,
    #[cfg(feature = "auto-update")]
    restart: std::rc::Rc<std::cell::RefCell<Option<sparkle_updater::RelaunchContinuation>>>,
    #[cfg(feature = "auto-update")]
    requested: std::rc::Rc<std::cell::Cell<bool>>,
    error: Option<String>,
}

impl Updates {
    pub fn new(_demo: bool) -> Self {
        #[allow(unused_mut)] // Mutable only when the optional native updater is compiled in.
        let mut updates = Self {
            #[cfg(feature = "auto-update")]
            updater: None,
            #[cfg(feature = "auto-update")]
            restart: Default::default(),
            #[cfg(feature = "auto-update")]
            requested: Default::default(),
            error: Some("Automatic updates are available in the signed release app, not demo or development builds.".into()),
        };
        #[cfg(feature = "auto-update")]
        if !_demo {
            use sparkle_updater::{MainThreadMarker, SparkleUpdater, UpdaterConfig};
            let restart = updates.restart.clone();
            let requested = updates.requested.clone();
            let config = UpdaterConfig {
                relaunch_handler: Some(std::rc::Rc::new(move |_, continuation| {
                    *restart.borrow_mut() = Some(continuation);
                    requested.set(true);
                })),
                ..Default::default()
            };
            match SparkleUpdater::new(MainThreadMarker::new().expect("GPUI main thread"), config) {
                Ok(Some(updater)) => { updates.updater = Some(updater); updates.error = None; }
                Ok(None) => {}
                Err(_) => updates.error = Some("Automatic updates could not start. Download the latest signed app from GitHub Releases.".into()),
            }
        }
        updates
    }

    pub fn available(&self) -> bool {
        self.error.is_none()
    }

    pub fn automatic_checks(&self) -> bool {
        #[cfg(feature = "auto-update")]
        {
            self.updater
                .as_ref()
                .and_then(|u| u.automatically_checks_for_updates().ok())
                .unwrap_or(false)
        }
        #[cfg(not(feature = "auto-update"))]
        {
            false
        }
    }

    pub fn set_automatic_checks(&self, _enabled: bool) -> Result<(), String> {
        #[cfg(feature = "auto-update")]
        if let Some(updater) = &self.updater {
            return updater
                .set_automatically_checks_for_updates(_enabled)
                .map_err(|_| "Could not change automatic update checking.".into());
        }
        Err("Automatic updates are unavailable in this build.".into())
    }

    pub fn check(&self) -> Result<(), String> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        #[cfg(feature = "auto-update")]
        if let Some(updater) = &self.updater {
            return updater
                .check_for_updates()
                .map_err(|_| "Could not check for updates. Please try again.".into());
        }
        Ok(())
    }

    pub fn restart_pending(&self) -> bool {
        #[cfg(feature = "auto-update")]
        {
            self.restart.borrow().is_some()
        }
        #[cfg(not(feature = "auto-update"))]
        {
            false
        }
    }

    pub fn take_restart_request(&self) -> bool {
        #[cfg(feature = "auto-update")]
        {
            self.requested.replace(false)
        }
        #[cfg(not(feature = "auto-update"))]
        {
            false
        }
    }

    /// Called only after the encrypted snapshot and settings have committed successfully.
    pub fn resume_restart(&self) -> bool {
        #[cfg(feature = "auto-update")]
        {
            let continuation = self.restart.borrow_mut().take();
            if let Some(continuation) = continuation {
                continuation
                    .resume(sparkle_updater::MainThreadMarker::new().expect("GPUI main thread"));
                return true;
            }
        }
        false
    }
}
