"use strict";

// Tiny task-list runner used as the corpus tier4 source program.
// Deliberately de-sugared ES5: var only, no arrow fns, no classes,
// no template literals, no destructuring, no spread, no optional chaining.
// Deterministic: no Date.now / Math.random / I/O beyond final console.log.

function makeStore(initial) {
    var items = [];
    var nextId = 1;
    var i = 0;
    while (i < initial.length) {
        items.push({ id: nextId, text: initial[i], done: false });
        nextId = nextId + 1;
        i = i + 1;
    }
    return {
        add: function (text) {
            var item = { id: nextId, text: text, done: false };
            items.push(item);
            nextId = nextId + 1;
            return item.id;
        },
        complete: function (id) {
            var j = 0;
            while (j < items.length) {
                if (items[j].id === id) {
                    items[j].done = true;
                    return true;
                }
                j = j + 1;
            }
            return false;
        },
        remove: function (id) {
            var kept = [];
            var k = 0;
            var n = 0;
            while (k < items.length) {
                if (items[k].id !== id) {
                    kept[n] = items[k];
                    n = n + 1;
                }
                k = k + 1;
            }
            items = kept;
        },
        list: function () {
            return items;
        }
    };
}

function parseCommand(line) {
    if (line.length === 0) {
        return { op: "", arg: "" };
    }
    var space = line.indexOf(" ");
    if (space < 0) {
        return { op: line, arg: "" };
    }
    return { op: line.substring(0, space), arg: line.substring(space + 1) };
}

function dispatch(store, cmd) {
    if (cmd.op === "add") {
        var text = cmd.arg.length > 0 ? cmd.arg : "(untitled)";
        var id = store.add(text);
        return "added #" + id + " " + text;
    } else if (cmd.op === "done") {
        var ok = store.complete(parseInt(cmd.arg, 10));
        return ok ? "done " + cmd.arg : "missing " + cmd.arg;
    } else if (cmd.op === "del") {
        store.remove(parseInt(cmd.arg, 10));
        return "del " + cmd.arg;
    } else if (cmd.op === "show") {
        var items = store.list();
        var lines = [];
        var p = 0;
        for (var q = 0; q < items.length; q = q + 1) {
            var it = items[q];
            lines[p] = (it.done ? "[x] " : "[ ] ") + it.id + " " + it.text;
            p = p + 1;
        }
        return lines.length === 0 ? "(empty)" : lines.join("\n");
    } else {
        return "unknown " + cmd.op;
    }
}

function runScript(store, lines) {
    var out = [];
    var w = 0;
    for (var i = 0; i < lines.length; i = i + 1) {
        var cmd = parseCommand(lines[i]);
        if (cmd.op === "") {
            return out;
        }
        out[w] = dispatch(store, cmd);
        w = w + 1;
    }
    return out;
}

function formatReport(outputs) {
    var visible = outputs.filter(function (s) {
        return s.length > 0;
    });
    var report = "";
    var idx = 0;
    while (idx < visible.length) {
        report = report + (idx === 0 ? "" : "\n") + (idx + 1) + ". " + visible[idx];
        idx = idx + 1;
    }
    return report;
}

var store = makeStore(["buy milk", "walk dog"]);
var out = runScript(store, [
    "add read book",
    "done 1",
    "del 2",
    "show",
    "",
    "add ignored"
]);
console.log(formatReport(out));
