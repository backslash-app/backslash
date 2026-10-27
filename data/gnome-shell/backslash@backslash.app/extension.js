// Lets the Backslash launcher list windows and switch to them: GNOME on
// Wayland gives apps no other way. Exposes two D-Bus methods on GNOME
// Shell's own connection, at /app/backslash/Windows.

import Gio from 'gi://Gio';
import Shell from 'gi://Shell';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const IFACE = `<node>
  <interface name="app.backslash.Windows">
    <method name="List"><arg type="s" direction="out" name="windows"/></method>
    <method name="Activate"><arg type="u" direction="in" name="id"/></method>
  </interface>
</node>`;

function windows() {
    return global.get_window_actors()
        .map(actor => actor.get_meta_window())
        .filter(w => w && !w.is_skip_taskbar());
}

export default class BackslashExtension extends Extension {
    enable() {
        this._dbus = Gio.DBusExportedObject.wrapJSObject(IFACE, this);
        this._dbus.export(Gio.DBus.session, '/app/backslash/Windows');
    }

    disable() {
        this._dbus?.unexport();
        this._dbus = null;
    }

    List() {
        const tracker = Shell.WindowTracker.get_default();
        return JSON.stringify(windows().map(w => ({
            id: w.get_id(),
            app_id: tracker.get_window_app(w)?.get_id()?.replace(/\.desktop$/, '') ?? '',
            wm_class: w.get_wm_class() ?? '',
            title: w.get_title() ?? '',
            focus: w.has_focus(),
            workspace: w.get_workspace()?.index() ?? -1,
        })));
    }

    Activate(id) {
        const w = windows().find(win => win.get_id() === id);
        if (!w)
            return;
        const time = global.get_current_time();
        const workspace = w.get_workspace();
        if (workspace)
            workspace.activate_with_focus(w, time);
        else
            w.activate(time);
    }
}
