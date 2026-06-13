// SPDX-License-Identifier: GPL-3.0-or-later
//
// Phosphor Scope — a live vectorscope applet for the Cinnamon panel.
//
// The applet itself is just a thin drawer: a small Python helper
// (phosphor_applet_feed.py) captures the default output monitor and streams
// beam segments as JSON lines on stdout, using the exact same signal path as
// the full Phosphor app. We read those lines and paint them with Cairo, in
// the panel and in a larger hover popup.

const Applet = imports.ui.applet;
const St = imports.gi.St;
const Cairo = imports.cairo;
const GLib = imports.gi.GLib;
const Gio = imports.gi.Gio;
const Mainloop = imports.mainloop;
const PopupMenu = imports.ui.popupMenu;
const Settings = imports.ui.settings;
const Util = imports.misc.util;

const UUID = "phosphor-scope@phosphor";

// Beam colours mirrored from phosphor_settings.THEME_PRESETS (each ×255).
const PHOSPHOR_COLOURS = {
    "P7 Green":     [107, 255, 140],
    "Amber":        [255, 158, 31],
    "Ice Blue":     [89, 191, 255],
    "White":        [235, 242, 255],
    "Vaporwave":    [255, 77, 224],
    "Red Phosphor": [255, 56, 41],
    "Ultraviolet":  [158, 102, 255],
    "Solar Gold":   [255, 214, 77],
    "Cyan Tube":    [51, 255, 235]
};

const MODES = [
    ["xy", "XY · scope art"],
    ["xy45", "Goniometer"],
    ["xy_dots", "XY · dots"],
    ["waveform", "Waveform"],
    ["spectrum", "Spectrum"],
    ["spectrum_radial", "Spectrum · radial"]
];

const TRAIL_FRAMES = 5;   // recent frames kept for a phosphor-style trail (kills flicker)

function roundedRectPath(cr, x, y, w, h, r) {
    r = Math.min(r, w / 2, h / 2);
    cr.newSubPath();
    cr.arc(x + w - r, y + r, r, -Math.PI / 2, 0);
    cr.arc(x + w - r, y + h - r, r, 0, Math.PI / 2);
    cr.arc(x + r, y + h - r, r, Math.PI / 2, Math.PI);
    cr.arc(x + r, y + r, r, Math.PI, 1.5 * Math.PI);
    cr.closePath();
}

function PhosphorScopeApplet(metadata, orientation, panelHeight, instanceId) {
    this._init(metadata, orientation, panelHeight, instanceId);
}

PhosphorScopeApplet.prototype = {
    __proto__: Applet.Applet.prototype,

    _init: function(metadata, orientation, panelHeight, instanceId) {
        Applet.Applet.prototype._init.call(this, orientation, panelHeight, instanceId);

        this._metadata = metadata;
        this._areaHeight = panelHeight;   // _panelHeight is a getter-only prop on the base applet
        this._frameHistory = [];      // recent frames of segments, for a phosphor-style trail
        this._closeTimerId = 0;

        this.settings = new Settings.AppletSettings(this, UUID, instanceId);
        this.settings.bind("colorMode", "colorMode", () => this._repaintAll());
        this.settings.bind("phosphorTheme", "phosphorTheme", () => this._repaintAll());
        this.settings.bind("background", "background", () => this._repaintAll());
        this.settings.bind("panelWidth", "panelWidth", () => this._applyPanelSize());
        this.settings.bind("squareInPanel", "squareInPanel", () => this._applyPanelSize());
        this.settings.bind("mode", "mode", () => this._onModeSetting());
        this.settings.bind("fps", "fps", () => this._restartFeed());

        this._panelArea = new St.DrawingArea({ style_class: "phosphor-panel-scope" });
        this._panelArea.connect("repaint", (area) => this._paint(area, false));
        this.actor.add_actor(this._panelArea);
        this._applyPanelSize();

        this._buildMenu(orientation);
        this._startFeed();
    },

    // -- sizing --------------------------------------------------------------

    _applyPanelSize: function() {
        let height = Math.max(16, this._areaHeight - 4);
        let width = this.squareInPanel ? height : this.panelWidth;
        this._panelArea.set_size(width, height);
        this._panelArea.queue_repaint();
    },

    on_panel_height_changed: function() {
        this._areaHeight = this._panelHeight;   // read the live base getter
        this._applyPanelSize();
    },

    // -- menu ----------------------------------------------------------------

    _buildMenu: function(orientation) {
        this.menu = new Applet.AppletPopupMenu(this, orientation);
        this.menuManager = new PopupMenu.PopupMenuManager(this);
        this.menuManager.addMenu(this.menu);

        let canvasItem = new PopupMenu.PopupBaseMenuItem({ reactive: false, activate: false });
        this._popupArea = new St.DrawingArea({ style_class: "phosphor-popup-scope" });
        this._popupArea.set_size(240, 240);
        this._popupArea.connect("repaint", (area) => this._paint(area, true));
        canvasItem.actor.add_actor(this._popupArea);
        this.menu.addMenuItem(canvasItem);

        this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());

        this._modeItems = {};
        MODES.forEach(([id, label]) => {
            let item = new PopupMenu.PopupMenuItem(label);
            item._phosphorLabel = label;
            item.connect("activate", () => this._setMode(id));
            this._modeItems[id] = item;
            this.menu.addMenuItem(item);
        });
        this._refreshModeMarks();

        this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());

        let openItem = new PopupMenu.PopupMenuItem("Open Phosphor");
        openItem.connect("activate", () => Util.spawnCommandLine("phosphor"));
        this.menu.addMenuItem(openItem);

        let pinItem = new PopupMenu.PopupMenuItem("Pin floating preview");
        pinItem.connect("activate", () => Util.spawnCommandLine("phosphor --mini"));
        this.menu.addMenuItem(pinItem);

        // Hover to reveal, with a small grace period so moving into the popup
        // doesn't dismiss it.
        this.actor.connect("enter-event", () => this._hoverOpen());
        this.actor.connect("leave-event", () => this._scheduleClose());
        this.menu.actor.connect("enter-event", () => this._cancelClose());
        this.menu.actor.connect("leave-event", () => this._scheduleClose());
    },

    on_applet_clicked: function() {
        this._cancelClose();
        this.menu.toggle();
    },

    _hoverOpen: function() {
        this._cancelClose();
        if (!this.menu.isOpen) this.menu.open();
    },

    _scheduleClose: function() {
        this._cancelClose();
        this._closeTimerId = Mainloop.timeout_add(350, () => {
            this._closeTimerId = 0;
            this.menu.close();
            return false;
        });
    },

    _cancelClose: function() {
        if (this._closeTimerId) {
            Mainloop.source_remove(this._closeTimerId);
            this._closeTimerId = 0;
        }
    },

    _setMode: function(id) {
        this.mode = id;
        this.settings.setValue("mode", id);
        this._refreshModeMarks();
        this._sendMode();   // programmatic setValue may not fire the bound callback, so send directly
    },

    _onModeSetting: function() {
        this._refreshModeMarks();
        this._sendMode();
    },

    _refreshModeMarks: function() {
        if (!this._modeItems) return;
        for (let id in this._modeItems) {
            let item = this._modeItems[id];
            let active = (id === this.mode);
            item.label.text = (active ? "● " : "    ") + item._phosphorLabel;
        }
    },

    // -- the feed subprocess -------------------------------------------------

    _helperPath: function() {
        let bundled = this._metadata.path + "/phosphor_applet_feed.py";
        if (GLib.file_test(bundled, GLib.FileTest.EXISTS)) return bundled;
        return "/usr/lib/phosphor/phosphor_applet_feed.py";
    },

    _startFeed: function() {
        this._cancellable = new Gio.Cancellable();
        try {
            let launcher = new Gio.SubprocessLauncher({
                flags: Gio.SubprocessFlags.STDIN_PIPE
                     | Gio.SubprocessFlags.STDOUT_PIPE
                     | Gio.SubprocessFlags.STDERR_SILENCE
            });
            this._proc = launcher.spawnv(["python3", this._helperPath(), "--fps", String(this.fps)]);
        } catch (e) {
            global.logError("[phosphor-scope] could not start feed: " + e);
            return;
        }
        this._stdout = new Gio.DataInputStream({ base_stream: this._proc.get_stdout_pipe() });
        this._stdin = new Gio.DataOutputStream({ base_stream: this._proc.get_stdin_pipe() });
        this._sendMode();
        this._readNextLine();
    },

    _readNextLine: function() {
        if (!this._stdout) return;
        this._stdout.read_line_async(GLib.PRIORITY_DEFAULT, this._cancellable, (stream, result) => {
            let line = null;
            try {
                [line] = stream.read_line_finish_utf8(result);
            } catch (e) {
                return;   // cancelled or stream gone
            }
            if (line === null) return;   // EOF
            this._onFeedLine(line);
            this._readNextLine();
        });
    },

    _onFeedLine: function(line) {
        let data;
        try {
            data = JSON.parse(line);
        } catch (e) {
            return;
        }
        if (data.error) {
            global.logError("[phosphor-scope] feed: " + data.error);
            return;
        }
        this._frameHistory.push(data.s || []);
        if (this._frameHistory.length > TRAIL_FRAMES) this._frameHistory.shift();
        this._panelArea.queue_repaint();
        if (this.menu && this.menu.isOpen && this._popupArea) {
            this._popupArea.queue_repaint();
        }
    },

    _sendMode: function() {
        if (!this._stdin) return;
        try {
            this._stdin.put_string("mode " + this.mode + "\n", null);
            this._stdin.flush(null);
        } catch (e) {
            // helper not ready yet, or pipe gone; harmless
        }
    },

    // -- drawing -------------------------------------------------------------

    _traceColour: function() {
        if (this.colorMode === "phosphor") {
            return PHOSPHOR_COLOURS[this.phosphorTheme] || PHOSPHOR_COLOURS["P7 Green"];
        }
        let colour = this.actor.get_theme_node().get_foreground_color();
        return [colour.red, colour.green, colour.blue];
    },

    _repaintAll: function() {
        if (this._panelArea) this._panelArea.queue_repaint();
        if (this._popupArea) this._popupArea.queue_repaint();
    },

    _paint: function(area, isPopup) {
        let cr = area.get_context();
        let [width, height] = area.get_surface_size();
        let rgb = this._traceColour();
        let r = rgb[0] / 255, g = rgb[1] / 255, b = rgb[2] / 255;

        cr.setOperator(Cairo.Operator.CLEAR);
        cr.paint();
        cr.setOperator(Cairo.Operator.OVER);

        // Optional standout background: a rounded panel in AMOLED black or a
        // dark tint of the trace colour, framed by a faint border in the trace
        // colour, so the scope reads as its own little instrument. The hover
        // popup always gets one (never fully transparent) for readability.
        let bgStyle = (isPopup && this.background === "transparent") ? "amoled" : this.background;
        if (bgStyle && bgStyle !== "transparent") {
            let radius = Math.max(3, Math.min(width, height) * 0.12);
            roundedRectPath(cr, 0.5, 0.5, width - 1, height - 1, radius);
            if (bgStyle === "amoled") cr.setSourceRGBA(0, 0, 0, 0.82);
            else cr.setSourceRGBA(r * 0.07, g * 0.07, b * 0.07, 0.85);   // themed dark tint
            cr.fillPreserve();
            cr.setSourceRGBA(r, g, b, 0.28);
            cr.setLineWidth(1);
            cr.stroke();
        }

        let history = this._frameHistory;
        let hasData = false;
        for (let h = 0; h < history.length; h++) {
            if (history[h].length >= 5) { hasData = true; break; }
        }
        if (!hasData) {
            // Silence / suspended sink: a faint resting dot.
            cr.setSourceRGBA(r, g, b, 0.45);
            cr.arc(width / 2, height / 2, Math.max(1, height * 0.03), 0, 2 * Math.PI);
            cr.fill();
            cr.$dispose();
            return;
        }

        let sx = width / 1000, sy = height / 1000;
        cr.setLineCap(Cairo.LineCap.ROUND);
        cr.setLineJoin(Cairo.LineJoin.ROUND);
        let glowWidth = isPopup ? 3.0 : 2.2;
        let coreWidth = isPopup ? 1.2 : 1.0;

        // Draw recent frames oldest-to-newest with rising brightness, so the
        // trace leaves a short phosphor trail instead of flickering frame to
        // frame. Each frame gets a soft wide glow then a bright thin core.
        for (let h = 0; h < history.length; h++) {
            let frame = history[h];
            if (frame.length < 5) continue;
            let age = (h + 1) / history.length;   // newest frame = 1.0
            let passes = [[glowWidth, 0.16 * age], [coreWidth, 0.92 * age]];
            for (let p = 0; p < passes.length; p++) {
                cr.setLineWidth(passes[p][0]);
                cr.setSourceRGBA(r, g, b, passes[p][1]);
                for (let i = 0; i + 5 <= frame.length; i += 5) {
                    cr.moveTo(frame[i] * sx, frame[i + 1] * sy);
                    cr.lineTo(frame[i + 2] * sx, frame[i + 3] * sy);
                }
                cr.stroke();
            }
        }
        cr.$dispose();
    },

    // -- teardown ------------------------------------------------------------

    _stopFeed: function() {
        if (this._cancellable) { this._cancellable.cancel(); this._cancellable = null; }
        try {
            if (this._stdin) { this._stdin.put_string("quit\n", null); this._stdin.flush(null); }
        } catch (e) {}
        try {
            if (this._proc) this._proc.force_exit();
        } catch (e) {}
        this._proc = null;
        this._stdin = null;
        this._stdout = null;
    },

    _restartFeed: function() {
        this._stopFeed();
        this._startFeed();
    },

    on_applet_removed_from_panel: function() {
        this._cancelClose();
        this._stopFeed();
        if (this.settings) this.settings.finalize();
    }
};

function main(metadata, orientation, panelHeight, instanceId) {
    return new PhosphorScopeApplet(metadata, orientation, panelHeight, instanceId);
}
