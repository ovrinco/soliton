pragma Singleton
import QtQuick
import Quickshell
import Quickshell.Io

QtObject {
    id: root
    property int sequence: 0
    property var callbacks: ({})
    property bool ready: false
    property var queued: []
    signal changed(string domain)
    property var changes: ({})
    property Timer debounce: Timer {
        interval: 180
        onTriggered: {
            var domains = Object.keys(root.changes); root.changes = ({});
            domains.forEach(domain => root.changed(domain));
        }
    }
    property Process events: Process {
        command: ['python3', Quickshell.env('SOLITON_RESOURCE_ROOT') + '/scripts/settings_events.py']
        stdout: SplitParser {
            onRead: data => {
                try { JSON.parse(data).forEach(domain => root.changes[domain] = true); root.debounce.restart(); }
                catch (error) { console.warn('Settings event:', error); }
            }
        }
    }
    property Process bridge: Process {
        command: ['python3', Quickshell.env('SOLITON_RESOURCE_ROOT') + '/scripts/settings_bridge.py']
        stdinEnabled: true
        onStarted: { root.ready = true; root.queued.forEach(line => root.bridge.write(line)); root.queued = []; }
        stdout: SplitParser {
            onRead: data => {
                try {
                    var response = JSON.parse(data), callback = root.callbacks[response.id];
                    delete root.callbacks[response.id];
                    if (callback) callback(response.result || {}, response.error || '');
                } catch (error) { console.warn('Settings response:', error); }
            }
        }
        onExited: {
            root.ready = false;
            var pending = root.callbacks; root.callbacks = ({}); root.queued = [];
            Object.keys(pending).forEach(key => pending[key]({}, 'Settings owner stopped'));
        }
    }
    // The desktop's monitors and bindings are the engine's; plugins and skills
    // stay with the bridge. A reviewed arrangement is kept by its token until
    // it is applied.
    property var desktopPlans: ({})
    function desktop(args, callback) {
        var answer = (map) => (result, ok, failure) => callback(ok ? map(result || {}) : {}, ok ? '' : (failure || 'The desktop could not be read'));
        var same = result => result;
        if (args.action === 'monitors') { Backend.send('desktop-monitors', {}, answer(same), true); return true; }
        if (args.action === 'bindings') { Backend.send('desktop-bindings', {}, answer(same), true); return true; }
        if (args.action === 'preview' && (args.kind === 'monitor_topology' || args.kind === 'monitor')) {
            var plan = Object.assign({}, args); delete plan.action; delete plan.kind; delete plan.id; delete plan.activeKey;
            Backend.send('desktop-arrangement-preview', plan, answer(result => {
                var plans = Object.assign({}, root.desktopPlans); plans[result.token] = plan; root.desktopPlans = plans;
                return result;
            }), true);
            return true;
        }
        if (args.action === 'preview' && args.kind === 'monitor_policy') { callback({}, 'This fallback form is retired'); return true; }
        if (args.action === 'apply' && root.desktopPlans[args.token]) {
            var chosen = root.desktopPlans[args.token];
            var rest = Object.assign({}, root.desktopPlans); delete rest[args.token]; root.desktopPlans = rest;
            Backend.send('desktop-arrangement-apply', chosen, answer(receipt => ({saved: receipt.written, applied: receipt.applied,
                confirmation: receipt.token, confirmSeconds: receipt.confirm_seconds})), true);
            return true;
        }
        if (args.action === 'confirm') {
            Backend.send('desktop-arrangement-confirm', {token: args.token, keep: !!args.keep}, answer(result => ({kept: result.kept})), true);
            return true;
        }
        return false;
    }
    function request(args, callback) {
        if (!events.running) events.running = true;
        if (desktop(args, callback)) return;
        var id = ++sequence, request = Object.assign({}, args, {id: id});
        callbacks[id] = callback;
        var line = JSON.stringify(request) + '\n';
        if (ready) bridge.write(line);
        else { queued.push(line); bridge.running = true; }
    }
}
