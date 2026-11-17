//! Labels, comments and questions, in English and French.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    En,
    Fr,
}

impl Lang {
    /// French when `$LANG` says so, English otherwise.
    pub fn from_env() -> Lang {
        match std::env::var("LANG") {
            Ok(lang) if lang.to_ascii_lowercase().starts_with("fr") => Lang::Fr,
            _ => Lang::En,
        }
    }

    pub fn text(self) -> &'static Text {
        match self {
            Lang::En => &EN,
            Lang::Fr => &FR,
        }
    }
}

pub struct Text {
    pub debug_header: &'static str,
    /// `{chip}` is replaced by the chip's name.
    pub esp_probe: &'static str,
    pub external_probe: &'static str,
    pub upload_debug: &'static str,
    pub build_debug: &'static str,
    pub attach: &'static str,
    pub upload_debug_note: &'static str,
    pub attach_note: &'static str,
    pub tasks_header: &'static str,
    pub free_probe_label: &'static str,
    pub settings_note: &'static str,
    pub task_build: &'static str,
    pub task_upload: &'static str,
    pub task_upload_monitor: &'static str,
    pub task_monitor: &'static str,
    pub task_clean: &'static str,
    pub task_compiledb: &'static str,
    pub task_test: &'static str,
    pub task_free_probe: &'static str,
    pub task_cargo_run: &'static str,
    pub q_env: &'static str,
    pub q_tasks: &'static str,
    pub q_tasks_pick: &'static str,
    pub q_upload_debug: &'static str,
    pub q_attach: &'static str,
    /// `{env}` is replaced by the environment's name.
    pub q_build_type: &'static str,
    /// `{name}` is replaced by the file name.
    pub q_overwrite: &'static str,
    pub q_chip: &'static str,
    pub q_program: &'static str,
    pub unchanged: &'static str,
    pub written: &'static str,
    pub skipped: &'static str,
    pub skipped_hint: &'static str,
    pub yes: &'static str,
    pub no: &'static str,
    pub next: &'static str,
}

impl Text {
    pub fn task(&self, key: &str) -> &'static str {
        match key {
            "build" => self.task_build,
            "upload" => self.task_upload,
            "upload_monitor" => self.task_upload_monitor,
            "monitor" => self.task_monitor,
            "clean" => self.task_clean,
            "compiledb" => self.task_compiledb,
            "test" => self.task_test,
            "free_probe" => self.task_free_probe,
            "cargo_run" => self.task_cargo_run,
            _ => "",
        }
    }
}

pub static EN: Text = Text {
    debug_header: "// Debug sessions for Zed (Zembed extension). Start one with F4.",
    esp_probe: "// The {chip} is debugged through its USB port: the same cable as for uploading.",
    external_probe: "// No built-in USB-JTAG on this chip: set the OpenOCD files of your probe.",
    upload_debug: "upload and debug",
    build_debug: "build, flash and debug",
    attach: "attach",
    upload_debug_note: "Builds, uploads, then stops in the running firmware.",
    attach_note: "Attaches to the firmware already on the board, without uploading.",
    tasks_header: "// Tasks (command palette: `task: spawn`, or Alt+Shift+T).",
    free_probe_label: "Debug: free the probe",
    settings_note: "clangd asks PlatformIO's compiler for its system headers.",
    task_build: "Builds the firmware.",
    task_upload: "Builds and uploads to the board.",
    task_upload_monitor: "Uploads, then opens the serial monitor (Ctrl+C to quit).",
    task_monitor: "Serial monitor.",
    task_clean: "Deletes the build files.",
    task_compiledb: "Regenerates compile_commands.json for code completion (clangd).",
    task_test: "Runs the unit tests.",
    task_free_probe:
        "If debugging cannot start because the probe is busy: stops leftover debug servers.",
    task_cargo_run: "Flashes and runs, with RTT logs.",
    q_env: "PlatformIO environment to set up",
    q_tasks: "Tasks",
    q_tasks_pick: "Numbers to keep, e.g. 1,2,4 (Enter = the ticked ones):",
    q_upload_debug: "Add an \"upload and debug\" session?",
    q_attach: "Add an \"attach without uploading\" session?",
    q_build_type:
        "Add `build_type = debug` to [env:{env}] in platformio.ini (keeps variables visible)?",
    q_overwrite: "Replace .zed/{name}? (a .bak copy is kept)",
    q_chip: "Chip as named by probe-rs (`probe-rs chip list`), e.g. STM32F411RETx",
    q_program: "Firmware ELF, e.g. target/thumbv7em-none-eabihf/debug/my-firmware",
    unchanged: "unchanged",
    written: "written",
    skipped: "kept as is",
    skipped_hint: "already exists, kept as is (--force to replace it, a .bak copy is kept)",
    yes: "y",
    no: "n",
    next: "Next: open the project in Zed, run `task: spawn` for the tasks, and F4 to debug.",
};

pub static FR: Text = Text {
    debug_header: "// Sessions de débogage pour Zed (extension Zembed). Lancer avec F4.",
    esp_probe: "// La sonde de l'{chip}, c'est son port USB : le même câble que pour téléverser.",
    external_probe: "// Pas d'USB-JTAG intégré sur cette puce : indiquer les fichiers OpenOCD de la sonde.",
    upload_debug: "upload et debug",
    build_debug: "build, flash et debug",
    attach: "attach",
    upload_debug_note: "Compile, téléverse, puis s'arrête dans le programme.",
    attach_note: "S'attache au programme déjà présent sur la carte, sans téléverser.",
    tasks_header: "// Tâches (palette de commandes : « task: spawn », ou Alt+Shift+T).",
    free_probe_label: "Debug: libérer la sonde",
    settings_note: "clangd interroge le compilateur de PlatformIO pour trouver ses en-têtes système.",
    task_build: "Compile le firmware.",
    task_upload: "Compile et téléverse sur la carte.",
    task_upload_monitor: "Téléverse puis ouvre le moniteur série (Ctrl+C pour quitter).",
    task_monitor: "Moniteur série.",
    task_clean: "Supprime les fichiers compilés.",
    task_compiledb: "Régénère compile_commands.json pour l'autocomplétion (clangd).",
    task_test: "Lance les tests unitaires.",
    task_free_probe: "Si le debug refuse de démarrer car la sonde est occupée : arrête les serveurs de debug restés ouverts.",
    task_cargo_run: "Flashe et lance, avec les logs RTT.",
    q_env: "Environnement PlatformIO à configurer",
    q_tasks: "Tâches",
    q_tasks_pick: "Numéros à garder, ex. 1,2,4 (Entrée = ceux cochés) :",
    q_upload_debug: "Ajouter une session « upload et debug » ?",
    q_attach: "Ajouter une session « attach » (sans téléverser) ?",
    q_build_type: "Ajouter `build_type = debug` à [env:{env}] dans platformio.ini (garde les variables visibles) ?",
    q_overwrite: "Remplacer .zed/{name} ? (une copie .bak est gardée)",
    q_chip: "Puce telle que probe-rs la nomme (`probe-rs chip list`), ex. STM32F411RETx",
    q_program: "Firmware ELF, ex. target/thumbv7em-none-eabihf/debug/mon-firmware",
    unchanged: "inchangé",
    written: "écrit",
    skipped: "laissé tel quel",
    skipped_hint: "existe déjà, laissé tel quel (--force pour le remplacer, une copie .bak est gardée)",
    yes: "o",
    no: "n",
    next: "Ensuite : ouvrir le projet dans Zed, « task: spawn » pour les tâches, F4 pour déboguer.",
};
