// postmini sandbox prelude: implements the Postman `pm` API on top of QuickJS.
// Native helpers provided by Rust: __dyn(name), __send(json), __jar(op, json), __lib(name), __sleep(ms)
globalThis.__setup = function (G) {
  'use strict';
  var CTX = G.__ctx;
  var OUT = { tests: [], console: [], next: undefined, skip: false, errors: [], visualizer: null };
  G.__out = OUT;

  // ------------------------------------------------------------ helpers
  function fmt(a) {
    if (typeof a === 'string') return a;
    if (a === undefined) return 'undefined';
    if (typeof a === 'function') return '[Function ' + (a.name || 'anonymous') + ']';
    if (a instanceof Error) return a.name + ': ' + a.message;
    try { return JSON.stringify(a, function (k, v) { return v === undefined ? '[undefined]' : (typeof v === 'function' ? '[Function]' : v); }, 2); } catch (e) { return String(a); }
  }
  function logger(level) {
    return function () {
      var parts = [];
      for (var i = 0; i < arguments.length; i++) parts.push(fmt(arguments[i]));
      var s = parts.join(' ');
      // printf-style %s %d %o
      if (typeof arguments[0] === 'string' && /%[sdoOij]/.test(arguments[0])) {
        var args = Array.prototype.slice.call(arguments, 1), idx = 0;
        s = arguments[0].replace(/%[sdoOij]/g, function (m) { if (idx >= args.length) return m; var v = args[idx++]; return m === '%d' || m === '%i' ? String(parseInt(v, 10)) : fmt(v); });
        for (; idx < args.length; idx++) s += ' ' + fmt(args[idx]);
      }
      OUT.console.push([level, s]);
    };
  }
  G.console = { log: logger('log'), info: logger('info'), warn: logger('warn'), error: logger('error'), debug: logger('debug'), trace: logger('log'), clear: function () { OUT.console.push(['clear', '']); }, time: function () {}, timeEnd: function () {} };

  // ------------------------------------------------------------ timers (virtual queue, real waiting)
  var timers = [], timerSeq = 1, now0 = Date.now();
  G.setTimeout = function (fn, ms) { var args = Array.prototype.slice.call(arguments, 2); var id = timerSeq++; timers.push({ id: id, at: Date.now() + (ms || 0), fn: fn, args: args, iv: 0 }); return id; };
  G.setInterval = function (fn, ms) { var args = Array.prototype.slice.call(arguments, 2); var id = timerSeq++; timers.push({ id: id, at: Date.now() + (ms || 0), fn: fn, args: args, iv: Math.max(ms || 0, 1) }); return id; };
  G.setImmediate = function (fn) { return G.setTimeout.apply(null, [fn, 0].concat(Array.prototype.slice.call(arguments, 1))); };
  G.clearTimeout = G.clearInterval = G.clearImmediate = function (id) { timers = timers.filter(function (t) { return t.id !== id; }); };
  G.__runNextTimer = function () {
    if (!timers.length) return false;
    timers.sort(function (a, b) { return a.at - b.at; });
    var t = timers.shift();
    var wait = t.at - Date.now();
    if (wait > 0) __sleep(wait);
    if (t.iv) { t.at = Date.now() + t.iv; timers.push(t); }
    try { if (typeof t.fn === 'function') t.fn.apply(null, t.args); } catch (e) { reportError(e); }
    return true;
  };

  function reportError(e) {
    var msg = (e && e.name ? e.name + ': ' : '') + (e && e.message !== undefined ? e.message : String(e));
    OUT.errors.push(msg);
  }
  G.__reportError = reportError;

  // ------------------------------------------------------------ libraries (lazy)
  var libCache = G.__libCache || (G.__libCache = {});
  function loadLib(name) {
    if (libCache[name]) return libCache[name];
    var src = __lib(name);
    if (src === undefined || src === null) throw new Error("Cannot find module '" + name + "'");
    var module = { exports: {} };
    var fn = (0, eval)('(function (module, exports, define, require, global, window, self) {' + src + '\n})');
    fn.call(G, module, module.exports, undefined, sandboxRequire, G, undefined, G);
    libCache[name] = module.exports;
    return module.exports;
  }
  var builtins = {
    'uuid': function () { var f = function () { return __dyn('$guid'); }; return { v4: f, v1: f, default: f }; },
    'atob': function () { return G.atob; },
    'btoa': function () { return G.btoa; },
    'xml2js': function () { return { parseString: function (x, o, cb) { if (typeof o === 'function') cb = o; try { cb(null, xml2Json(x)); } catch (e) { cb(e); } } }; },
    'csv-parse/lib/sync': function () { return csvParse; },
    'csv-parse/sync': function () { return { parse: csvParse }; },
    'buffer': function () { return { Buffer: G.Buffer }; },
    'url': function () { return { URL: G.URL, URLSearchParams: G.URLSearchParams }; },
    'querystring': function () { return { parse: function (s) { var o = {}; String(s).split('&').forEach(function (p) { if (!p) return; var i = p.indexOf('='); var k = decodeURIComponent((i < 0 ? p : p.slice(0, i)).replace(/\+/g, ' ')); var v = i < 0 ? '' : decodeURIComponent(p.slice(i + 1).replace(/\+/g, ' ')); o[k] = v; }); return o; }, stringify: function (o) { return Object.keys(o || {}).map(function (k) { return encodeURIComponent(k) + '=' + encodeURIComponent(o[k]); }).join('&'); } }; }
  };
  var libNames = { 'chai': 'chai', 'lodash': 'lodash', 'crypto-js': 'crypto-js', 'moment': 'moment', 'ajv': 'ajv', 'tv4': 'tv4' };
  function sandboxRequire(name) {
    if (libNames[name]) return loadLib(libNames[name]);
    if (builtins[name]) { if (!libCache['b:' + name]) libCache['b:' + name] = builtins[name](); return libCache['b:' + name]; }
    throw new Error("Cannot find module '" + name + "' (not available in postmini sandbox)");
  }
  G.require = sandboxRequire;
  function lazyGlobal(name, getter) { Object.defineProperty(G, name, { configurable: true, enumerable: false, get: function () { Object.defineProperty(G, name, { value: undefined, writable: true, configurable: true }); var v = getter(); Object.defineProperty(G, name, { value: v, writable: true, configurable: true }); return v; } }); }
  lazyGlobal('_', function () { return loadLib('lodash'); });
  lazyGlobal('CryptoJS', function () { return loadLib('crypto-js'); });
  lazyGlobal('tv4', function () { return loadLib('tv4'); });
  lazyGlobal('moment', function () { return loadLib('moment'); });
  lazyGlobal('chai', function () { return getChai(); });

  // ------------------------------------------------------------ atob / btoa
  var B64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/=';
  G.btoa = function (input) {
    var str = String(input), out = '';
    for (var block, charCode, idx = 0, map = B64; str.charAt(idx | 0) || (map = '=', idx % 1); out += map.charAt(63 & block >> 8 - idx % 1 * 8)) {
      charCode = str.charCodeAt(idx += 3 / 4);
      if (charCode > 0xFF) throw new Error("'btoa' failed: The string to be encoded contains characters outside of the Latin1 range.");
      block = block << 8 | charCode;
    }
    return out;
  };
  G.atob = function (input) {
    var str = String(input).replace(/[=\s]+$/, ''), out = '';
    for (var bc = 0, bs, buffer, idx = 0; buffer = str.charAt(idx++); ~buffer && (bs = bc % 4 ? bs * 64 + buffer : buffer, bc++ % 4) ? out += String.fromCharCode(255 & bs >> (-2 * bc & 6)) : 0) {
      buffer = B64.indexOf(buffer);
    }
    return out;
  };

  // minimal Buffer for common uses: Buffer.from(str[, enc]).toString(enc)
  function utf8encode(s) { return unescape(encodeURIComponent(s)); }
  function utf8decode(s) { try { return decodeURIComponent(escape(s)); } catch (e) { return s; } }
  function Buf(bin) { this._b = bin; this.length = bin.length; }
  Buf.prototype.toString = function (enc) {
    enc = (enc || 'utf8').toLowerCase();
    if (enc === 'base64') return G.btoa(this._b);
    if (enc === 'hex') { var h = ''; for (var i = 0; i < this._b.length; i++) h += ('0' + this._b.charCodeAt(i).toString(16)).slice(-2); return h; }
    if (enc === 'binary' || enc === 'latin1' || enc === 'ascii') return this._b;
    return utf8decode(this._b);
  };
  Buf.prototype.toJSON = function () { var d = []; for (var i = 0; i < this._b.length; i++) d.push(this._b.charCodeAt(i)); return { type: 'Buffer', data: d }; };
  G.Buffer = {
    from: function (v, enc) {
      enc = (enc || 'utf8').toLowerCase();
      if (v instanceof Buf) return new Buf(v._b);
      if (Array.isArray(v)) return new Buf(String.fromCharCode.apply(null, v));
      v = String(v);
      if (enc === 'base64') return new Buf(G.atob(v.replace(/-/g, '+').replace(/_/g, '/')));
      if (enc === 'hex') { var s = ''; for (var i = 0; i < v.length; i += 2) s += String.fromCharCode(parseInt(v.substr(i, 2), 16)); return new Buf(s); }
      if (enc === 'binary' || enc === 'latin1' || enc === 'ascii') return new Buf(v);
      return new Buf(utf8encode(v));
    },
    isBuffer: function (b) { return b instanceof Buf; },
    byteLength: function (s) { return utf8encode(String(s)).length; },
    concat: function (l) { return new Buf(l.map(function (b) { return b._b; }).join('')); }
  };

  // ------------------------------------------------------------ PropertyList (headers, query, cookies...)
  function PList(items, opts) {
    this.members = (items || []).slice();
    this._ci = !opts || opts.ci !== false;
  }
  PList.prototype._eq = function (a, b) { return this._ci ? String(a).toLowerCase() === String(b).toLowerCase() : a === b; };
  PList.prototype._enabled = function () { return this.members.filter(function (m) { return !m.disabled; }); };
  PList.prototype.get = function (k) { var m = this.one(k); return m ? m.value : undefined; };
  PList.prototype.one = function (k) { var self = this; var r; this._enabled().forEach(function (m) { if (self._eq(m.key, k)) r = m; }); return r; };
  PList.prototype.has = function (k, v) {
    var self = this;
    if (typeof k === 'object' && k) { v = k.value; k = k.key; }
    return this._enabled().some(function (m) { return self._eq(m.key, k) && (v === undefined || m.value === v); });
  };
  PList.prototype.all = function () { return this.members.slice(); };
  PList.prototype.count = function () { return this.members.length; };
  PList.prototype.idx = function (i) { return this.members[i]; };
  PList.prototype.indexOf = function (k) { var self = this; for (var i = 0; i < this.members.length; i++) if (self._eq(this.members[i].key, typeof k === 'object' ? k.key : k)) return i; return -1; };
  PList.prototype.each = PList.prototype.forEach = function (fn, ctx) { this.members.forEach(function (m, i) { fn.call(ctx, m, i); }); };
  PList.prototype.map = function (fn, ctx) { return this.members.map(fn, ctx); };
  PList.prototype.filter = function (fn, ctx) { return this.members.filter(fn, ctx); };
  PList.prototype.find = function (fn, ctx) { return this.members.find(fn, ctx); };
  PList.prototype.reduce = function (fn, acc) { return this.members.reduce(fn, acc); };
  PList.prototype.toObject = function (excludeDisabled, caseSensitive, multiValue) {
    var o = {};
    this.members.forEach(function (m) {
      if (excludeDisabled && m.disabled) return;
      var k = caseSensitive === false ? String(m.key).toLowerCase() : m.key;
      if (multiValue && o.hasOwnProperty(k)) { if (!Array.isArray(o[k])) o[k] = [o[k]]; o[k].push(m.value); } else o[k] = m.value;
    });
    return o;
  };
  PList.prototype.toJSON = function () { return this.members.map(function (m) { return { key: m.key, value: m.value }; }); };
  PList.prototype.toString = function () { return this.members.filter(function (m) { return !m.disabled; }).map(function (m) { return m.key + ': ' + m.value; }).join('\n'); };
  PList.prototype._norm = function (h, v) { if (typeof h === 'string') { if (v !== undefined) return { key: h, value: String(v) }; var i = h.indexOf(':'); return i < 0 ? { key: h, value: '' } : { key: h.slice(0, i).trim(), value: h.slice(i + 1).trim() }; } return { key: h.key, value: h.value === undefined ? '' : String(h.value), disabled: !!h.disabled }; };
  PList.prototype.add = PList.prototype.append = function (h, v) { this.members.push(this._norm(h, v)); this._changed && this._changed(); };
  PList.prototype.prepend = function (h) { this.members.unshift(this._norm(h)); this._changed && this._changed(); };
  PList.prototype.upsert = function (h, v) { var n = this._norm(h, v), self = this, found = false; this.members.forEach(function (m) { if (self._eq(m.key, n.key)) { m.value = n.value; m.disabled = false; found = true; } }); if (!found) this.members.push(n); this._changed && this._changed(); };
  PList.prototype.remove = function (k) { var self = this; if (typeof k === 'function') this.members = this.members.filter(function (m) { return !k(m); }); else { var key = typeof k === 'object' ? k.key : k; this.members = this.members.filter(function (m) { return !self._eq(m.key, key); }); } this._changed && this._changed(); };
  PList.prototype.clear = function () { this.members = []; this._changed && this._changed(); };
  PList.prototype.populate = function (arr) { var self = this; (arr || []).forEach(function (a) { self.members.push(self._norm(a)); }); this._changed && this._changed(); };
  PList.prototype.repopulate = function (arr) { this.members = []; this.populate(arr); };
  PList.prototype.eachParent = function () {};

  // ------------------------------------------------------------ variable scopes
  function Scope(obj, name) {
    this._name = name;
    this.values = [];
    var self = this;
    Object.keys(obj || {}).forEach(function (k) { self.values.push({ key: k, value: obj[k] }); });
  }
  Scope.prototype._find = function (k) { for (var i = this.values.length - 1; i >= 0; i--) if (this.values[i].key === k) return this.values[i]; };
  Scope.prototype.has = function (k) { return !!this._find(k); };
  Scope.prototype.get = function (k) { var v = this._find(k); return v ? v.value : undefined; };
  Scope.prototype.set = function (k, v) {
    if (typeof k !== 'string') throw new TypeError('Variable key must be a string');
    var e = this._find(k);
    if (e) e.value = v; else this.values.push({ key: k, value: v });
  };
  Scope.prototype.unset = function (k) { this.values = this.values.filter(function (e) { return e.key !== k; }); };
  Scope.prototype.clear = function () { this.values = []; };
  Scope.prototype.toObject = function () { var o = {}; this.values.forEach(function (e) { o[e.key] = e.value; }); return o; };
  Scope.prototype.toJSON = function () { return { values: this.values.map(function (e) { return { key: e.key, value: e.value }; }) }; };
  Scope.prototype.replaceIn = function (s) { return replaceIn(s, [this]); };
  Scope.prototype.syncVariablesFrom = function (o) { this.values = []; var self = this; Object.keys(o || {}).forEach(function (k) { self.values.push({ key: k, value: o[k] }); }); };

  var locals = new Scope(CTX.vars.locals, 'local');
  var data = new Scope(CTX.vars.data, 'data');
  var environment = new Scope(CTX.vars.environment, 'environment');
  environment.name = CTX.vars.envName || undefined;
  var collectionVariables = new Scope(CTX.vars.collection, 'collection');
  var globals = new Scope(CTX.vars.globals, 'globals');
  var layers = [locals, data, environment, collectionVariables, globals];

  function lookup(k, ls) {
    for (var i = 0; i < ls.length; i++) { var e = ls[i]._find(k); if (e) return { v: e.value }; }
    if (k.charAt(0) === '$') { var d = __dyn(k); if (d !== undefined && d !== null) return { v: d }; }
  }
  function toStr(v) { if (v === null || v === undefined) return ''; if (typeof v === 'object') return JSON.stringify(v); return String(v); }
  function replaceIn(s, ls) {
    if (typeof s !== 'string') {
      if (s && typeof s === 'object') return JSON.parse(replaceIn(JSON.stringify(s), ls));
      return s;
    }
    for (var n = 0; n < 19; n++) {
      var replaced = false;
      s = s.replace(/{{([^{}]*?)}}/g, function (m, k) { var r = lookup(k, ls); if (r) { replaced = true; return toStr(r.v); } return m; });
      if (!replaced) break;
    }
    return s;
  }
  var variables = {
    get: function (k) { var r = lookup(k, layers.filter(function () { return true; })); return r ? r.v : undefined; },
    set: function (k, v) { locals.set(k, v); },
    has: function (k) { return layers.some(function (l) { return l.has(k); }); },
    unset: function (k) { locals.unset(k); },
    clear: function () { locals.clear(); },
    toObject: function () { var o = {}; for (var i = layers.length - 1; i >= 0; i--) { var t = layers[i].toObject(); for (var k in t) o[k] = t[k]; } return o; },
    replaceIn: function (s) { return replaceIn(s, layers); },
    toJSON: function () { return { values: Object.keys(this.toObject()).map(function (k) { return { key: k, value: variables.get(k) }; }) }; }
  };
  ['get', 'set', 'unset', 'clear', 'has', 'toObject'].forEach(function () {});
  // pm.environment/globals/collectionVariables replaceIn resolve with all layers (as in Postman)
  [environment, collectionVariables, globals].forEach(function (s) { s.replaceIn = function (str) { return replaceIn(str, layers); }; });
  var iterationData = new Scope(CTX.vars.data, 'data');
  iterationData.set = undefined;

  // ------------------------------------------------------------ URL
  function Url(raw) { this.update(raw); }
  Url.prototype.update = function (raw) {
    raw = String(raw || '');
    var hash = '', i = raw.indexOf('#');
    if (i >= 0) { hash = raw.slice(i + 1); raw = raw.slice(0, i); }
    var q = '', j = raw.indexOf('?');
    if (j >= 0) { q = raw.slice(j + 1); raw = raw.slice(0, j); }
    var proto = '', p = raw.indexOf('://');
    if (p >= 0) { proto = raw.slice(0, p); raw = raw.slice(p + 3); }
    var slash = raw.indexOf('/');
    var hostport = slash < 0 ? raw : raw.slice(0, slash);
    var path = slash < 0 ? '' : raw.slice(slash + 1);
    var auth = null, at = hostport.lastIndexOf('@');
    if (at >= 0) { auth = hostport.slice(0, at); hostport = hostport.slice(at + 1); }
    var port = '', c = hostport.lastIndexOf(':');
    if (c >= 0 && hostport.indexOf('{{') !== 0 && hostport.slice(c + 1).indexOf(']') < 0) { port = hostport.slice(c + 1); hostport = hostport.slice(0, c); }
    this.protocol = proto || undefined;
    this.auth = auth;
    this.host = hostport ? hostport.split('.') : [];
    this.port = port || undefined;
    this.path = path ? path.split('/') : [];
    this.hash = hash || undefined;
    var self = this;
    var qm = q ? q.split('&').filter(function (x) { return x !== ''; }).map(function (x) { var k = x.indexOf('='); return k < 0 ? { key: x, value: null } : { key: x.slice(0, k), value: x.slice(k + 1) }; }) : [];
    this.query = new PList(qm, { ci: false });
    this.variables = new PList((CTX.request && CTX.request.pathVariables) || [], { ci: false });
  };
  Url.prototype.getHost = function () { return this.host.join('.'); };
  Url.prototype.getPath = function (unresolved) { var p = '/' + this.path.join('/'); if (!unresolved) { var vs = this.variables; p = p.replace(/\/:([^\/]+)/g, function (m, n) { var v = vs.get(n); return v !== undefined && v !== '' ? '/' + v : m; }); } return p; };
  Url.prototype.getQueryString = function () { return this.query.members.filter(function (m) { return !m.disabled; }).map(function (m) { return m.value === null || m.value === undefined ? m.key : m.key + '=' + m.value; }).join('&'); };
  Url.prototype.getPathWithQuery = function () { var q = this.getQueryString(); return this.getPath() + (q ? '?' + q : ''); };
  Url.prototype.getRemote = function () { return this.getHost() + (this.port ? ':' + this.port : ''); };
  Url.prototype.getOAuth1BaseUrl = function () { return (this.protocol || 'http') + '://' + this.getRemote() + this.getPath(); };
  Url.prototype.toString = function () {
    var s = '';
    if (this.protocol) s += this.protocol + '://';
    if (this.auth) s += this.auth + '@';
    s += this.getHost();
    if (this.port) s += ':' + this.port;
    if (this.path.length) s += '/' + this.path.join('/');
    var q = this.getQueryString();
    if (q) s += '?' + q;
    if (this.hash) s += '#' + this.hash;
    return s;
  };
  Url.prototype.toJSON = function () { return this.toString(); };
  Url.prototype.addQueryParams = function (p) { var self = this; if (typeof p === 'string') p = p.split('&').map(function (x) { var k = x.indexOf('='); return k < 0 ? { key: x, value: null } : { key: x.slice(0, k), value: x.slice(k + 1) }; }); (Array.isArray(p) ? p : [p]).forEach(function (x) { self.query.add(x); }); };
  Url.prototype.removeQueryParams = function (p) { var self = this; (Array.isArray(p) ? p : [p]).forEach(function (x) { self.query.remove(typeof x === 'object' ? x.key : x); }); };
  Url.parse = function (s) { return new Url(s); };

  // ------------------------------------------------------------ Request
  function mkHeaders(list) { return new PList((list || []).map(function (h) { return { key: h.key, value: h.value, disabled: !!h.disabled }; })); }
  function Request(def) {
    def = def || {};
    if (typeof def === 'string') def = { url: def };
    this.id = def.id;
    this.name = def.name;
    this.method = (def.method || 'GET').toUpperCase();
    this.url = def.url instanceof Url ? def.url : new Url(typeof def.url === 'object' && def.url ? (def.url.raw || '') : def.url);
    var hs = def.header || def.headers || [];
    if (!Array.isArray(hs) && typeof hs === 'object' && !(hs instanceof PList)) hs = Object.keys(hs).map(function (k) { return { key: k, value: hs[k] }; });
    if (typeof hs === 'string') hs = hs.split('\n').filter(Boolean).map(function (l) { var i = l.indexOf(':'); return { key: l.slice(0, i).trim(), value: l.slice(i + 1).trim() }; });
    this.headers = hs instanceof PList ? hs : mkHeaders(hs);
    var b = def.body || null;
    if (b) {
      this.body = {
        mode: b.mode, raw: b.raw, graphql: b.graphql, file: b.file, options: b.options, disabled: b.disabled,
        urlencoded: new PList((b.urlencoded || []).map(function (x) { return { key: x.key, value: x.value, disabled: !!x.disabled }; }), { ci: false }),
        formdata: new PList((b.formdata || []).map(function (x) { return { key: x.key, value: x.value, disabled: !!x.disabled, type: x.type, src: x.src }; }), { ci: false })
      };
      this.body.toString = function () {
        if (this.mode === 'raw') return this.raw || '';
        if (this.mode === 'urlencoded') return this.urlencoded.members.filter(function (m) { return !m.disabled; }).map(function (m) { return encodeURIComponent(m.key) + '=' + encodeURIComponent(m.value); }).join('&');
        if (this.mode === 'graphql') return JSON.stringify(this.graphql || {});
        return '';
      };
      this.body.isEmpty = function () { return !this.mode || (this.mode === 'raw' && !this.raw) || (this.mode === 'urlencoded' && !this.urlencoded.count()) || (this.mode === 'formdata' && !this.formdata.count()); };
      this.body.update = function (o) { if (typeof o === 'string') { this.mode = 'raw'; this.raw = o; } else { for (var k in o) { if (k === 'urlencoded' || k === 'formdata') this[k].repopulate(o[k]); else this[k] = o[k]; } } };
    } else this.body = undefined;
    this.auth = def.auth;
  }
  Request.prototype.addHeader = function (h) { this.headers.add(h); };
  Request.prototype.upsertHeader = function (h) { this.headers.upsert(h); };
  Request.prototype.removeHeader = function (k) { this.headers.remove(typeof k === 'object' ? k.key : k); };
  Request.prototype.getHeaders = function (o) { var r = {}; this.headers.members.forEach(function (m) { if (o && o.ignoreDisabled && m.disabled) return; r[o && o.ignoreCase ? m.key.toLowerCase() : m.key] = m.value; }); return r; };
  Request.prototype.addQueryParams = function (p) { this.url.addQueryParams(p); };
  Request.prototype.removeQueryParams = function (p) { this.url.removeQueryParams(p); };
  Request.prototype.toJSON = function () { return exportRequest(this); };
  Request.prototype.size = function () { var b = this.body ? this.body.toString() : ''; return { body: b.length, header: this.headers.toString().length, total: b.length + this.headers.toString().length }; };
  Request.prototype.authorizeUsing = function (type, opts) { if (type === null) { this.auth = { type: 'noauth' }; return; } if (typeof type === 'object') { this.auth = type; return; } var a = { type: type }; a[type] = Object.keys(opts || {}).map(function (k) { return { key: k, value: opts[k], type: 'string' }; }); this.auth = a; };

  function exportRequest(r) {
    var o = { method: r.method, url: r.url.toString(), header: r.headers.members.map(function (m) { var h = { key: m.key, value: m.value }; if (m.disabled) h.disabled = true; return h; }) };
    if (r.body) {
      var b = { mode: r.body.mode };
      if (r.body.raw !== undefined) b.raw = r.body.raw;
      if (r.body.options) b.options = r.body.options;
      if (r.body.graphql) b.graphql = r.body.graphql;
      if (r.body.file) b.file = r.body.file;
      if (r.body.urlencoded.count()) b.urlencoded = r.body.urlencoded.members.map(function (m) { return { key: m.key, value: m.value, disabled: !!m.disabled }; });
      if (r.body.formdata.count()) b.formdata = r.body.formdata.members.map(function (m) { var x = { key: m.key, value: m.value, disabled: !!m.disabled }; if (m.type) x.type = m.type; if (m.src) x.src = m.src; return x; });
      o.body = b;
    }
    if (r.auth) o.auth = r.auth;
    return o;
  }

  // ------------------------------------------------------------ Response
  function Response(def) {
    this.id = def.id;
    this.code = def.code;
    this.status = def.status;
    this.headers = mkHeaders(def.header);
    this.responseTime = def.responseTime;
    this.responseSize = def.responseSize;
    this._body = def.body || '';
    this.cookies = new PList((def.cookies || []).map(function (c) { return { key: c.name, name: c.name, value: c.value, domain: c.domain, path: c.path, expires: c.expires, httpOnly: c.httpOnly, secure: c.secure }; }), { ci: false });
    var self = this;
    this.stream = { toString: function (enc) { return enc === 'base64' ? (def.bodyBase64 || G.btoa(utf8encode(self._body))) : self._body; }, length: def.responseSize };
  }
  Response.prototype.text = function () { return this._body; };
  Response.prototype.json = function (reviver) {
    var t = this._body.replace(/^﻿/, '');
    try { return JSON.parse(t, reviver); } catch (e) { var err = new SyntaxError('JSONError: ' + e.message + (t.length < 100 ? ' in "' + t + '"' : '')); err.name = 'JSONError'; throw err; }
  };
  Response.prototype.reason = function () { return this.status; };
  Response.prototype.size = function () { var h = this.headers.toString().length; return { body: this.responseSize, header: h, total: this.responseSize + h }; };
  Response.prototype.dataURI = function () { var ct = this.headers.get('content-type') || 'text/plain'; return 'data:' + ct + ';base64,' + this.stream.toString('base64'); };
  Response.prototype.contentInfo = function () { var ct = this.headers.get('content-type') || ''; return { contentType: ct, mimeType: ct.split('/')[0], mimeFormat: (ct.split('/')[1] || '').split(';')[0], charset: (/charset=([^;]+)/.exec(ct) || [])[1] || 'utf8' }; };
  Response.prototype.toJSON = function () { return { code: this.code, status: this.status, header: this.headers.toJSON(), responseTime: this.responseTime, body: this._body }; };

  // ------------------------------------------------------------ chai + postman assertions (subset of chai-postman)
  var chaiLib = G.__chaiLib;
  function getChai() {
    if (chaiLib) return chaiLib;
    chaiLib = loadLib('chai');
    G.__chaiLib = chaiLib;
    chaiLib.use(function (chai, utils) {
      var A = chai.Assertion;
      function isResp(o) { return !!o && o.__pmType === 'response'; }
      function isReq(o) { return !!o && o.__pmType === 'request'; }
      A.addProperty('postmanRequest', function () { this.assert(isReq(this._obj), 'expecting a postman request object but got #{this}', 'not expecting a postman request object'); });
      A.addProperty('postmanResponse', function () { this.assert(isResp(this._obj), 'expecting a postman response object but got #{this}', 'not expecting a postman response object'); });
      A.addProperty('postmanRequestOrResponse', function () { this.assert(isResp(this._obj) || isReq(this._obj), 'expecting a postman request or response object but got #{this}', 'not expecting a postman request or response object'); });
      A.addMethod('statusCodeClass', function (cls) {
        var c = this._obj.code, k = Math.floor(c / 100);
        this.assert(k === cls, 'expected response code to be ' + cls + 'XX but found ' + c, 'expected response code to not be ' + cls + 'XX but found ' + c);
      });
      var classes = { info: 1, success: 2, redirection: 3, clientError: 4, serverError: 5 };
      Object.keys(classes).forEach(function (n) {
        A.addProperty(n, function () { var c = this._obj.code, k = Math.floor(c / 100); this.assert(k === classes[n], 'expected response code to be ' + classes[n] + 'XX but found ' + c, 'expected response code to not be ' + classes[n] + 'XX but found ' + c); });
      });
      A.addProperty('error', function () { var c = this._obj.code, k = Math.floor(c / 100); this.assert(k === 4 || k === 5, 'expected response code to be 4XX or 5XX but found ' + c, 'expected response code to not be 4XX or 5XX but found ' + c); });
      var named = { accepted: 202, badRequest: 400, forbidden: 403, notFound: 404, rateLimited: 429, unauthorized: 401, unauthorised: 401 };
      Object.keys(named).forEach(function (n) {
        A.addProperty(n, function () { var c = this._obj.code; this.assert(c === named[n], 'expected response code to be ' + named[n] + ' but found ' + c, 'expected response code to not be ' + named[n] + ' but found ' + c); });
      });
      A.overwriteProperty('ok', function (_super) {
        return function () {
          if (isResp(this._obj)) { var c = this._obj.code; this.assert(c === 200, 'expected response code to be 200 but found ' + c, 'expected response code to not be 200 but found ' + c); }
          else _super.call(this);
        };
      });
      A.addMethod('statusCode', function (code) { var c = this._obj.code; this.assert(c === code, 'expected response to have status code #{exp} but got #{act}', 'expected response to not have status code #{act}', code, c); });
      A.addMethod('statusReason', function (r) { var s = this._obj.status; this.assert(s === r, 'expected response to have status reason #{exp} but got #{act}', 'expected response to not have status reason #{act}', r, s); });
      A.addMethod('status', function (cr) {
        var o = this._obj;
        if (typeof cr === 'number') this.assert(o.code === cr, 'expected response to have status code #{exp} but got #{act}', 'expected response to not have status code #{act}', cr, o.code);
        else this.assert(o.status === cr, 'expected response to have status reason #{exp} but got #{act}', 'expected response to not have status reason #{act}', cr, o.status);
      });
      A.overwriteMethod('header', function (_super) {
        return function (k, v) {
          if (!isResp(this._obj) && !isReq(this._obj)) return _super.apply(this, arguments);
          var h = this._obj.headers;
          if (arguments.length < 2) this.assert(h.has(k), "expected response to have header with key '" + k + "'", "expected response to not have header with key '" + k + "'", true, h.has(k));
          else this.assert(h.get(k) === v, "expected '" + h.get(k) + "' response header to be '" + v + "' but got '" + h.get(k) + "'", "expected response header '" + k + "' to not be '" + v + "'", v, h.get(k));
        };
      });
      A.addProperty('withBody', function () { var b = this._obj.text ? this._obj.text() : ''; this.assert(b && b.length > 0, 'expected response to have content in body', 'expected response to not have content in body'); });
      A.overwriteProperty('json', function (_super) {
        return function () {
          if (!isResp(this._obj)) return _super.call(this);
          var ok = true; try { this._obj.json(); } catch (e) { ok = false; }
          this.assert(ok, 'expected response body to be a valid json', 'expected response body not to be a valid json');
        };
      });
      A.addMethod('body', function (content) {
        var t = this._obj.text();
        if (arguments.length === 0) { this.assert(t && t.length, 'expected response to have content in body', 'expected response to not have content in body'); return; }
        if (typeof content === 'string') this.assert(t === content, 'expected response body to equal #{exp} but got #{act}', 'expected response body to not equal #{exp}', content, t);
        else if (content instanceof RegExp) this.assert(content.test(t), 'expected response body text to match #{exp}', 'expected response body text to not match #{exp}', content, t);
        else { var j; try { j = this._obj.json(); } catch (e) {} this.assert(utils.eql(j, content), 'expected response body json to equal #{exp} but got #{act}', 'expected response body json to not equal #{exp}', content, j); }
      });
      A.addMethod('jsonBody', function (path, value) {
        var j; try { j = this._obj.json(); } catch (e) {}
        if (arguments.length === 0) { this.assert(j !== undefined, 'expected response body to be a valid json', 'expected response body not to be a valid json'); return; }
        if (typeof path === 'object') { this.assert(utils.eql(j, path), 'expected response body json to equal #{exp} but got #{act}', 'expected response body json to not equal #{exp}', path, j); return; }
        var cur = j, parts = String(path).replace(/\[(\w+)\]/g, '.$1').split('.').filter(Boolean), found = true;
        for (var i = 0; i < parts.length; i++) { if (cur !== null && cur !== undefined && Object.prototype.hasOwnProperty.call(Object(cur), parts[i])) cur = cur[parts[i]]; else { found = false; break; } }
        if (arguments.length < 2) this.assert(found, "expected #{this} to have property '" + path + "'", "expected #{this} to not have property '" + path + "'");
        else this.assert(found && utils.eql(cur, value), "expected response body json at '" + path + "' to contain #{exp} but got #{act}", "expected response body json at '" + path + "' to not contain #{exp}", value, cur);
      });
      A.addChainableMethod('responseTime', function (v) { var t = this._obj.responseTime; if (arguments.length) this.assert(t === v, 'expected response to have response time #{exp} but got #{act}', 'expected response to not have response time #{exp}', v, t); }, function () { utils.flag(this, 'object', this._obj.responseTime); });
      A.addChainableMethod('responseSize', function (v) { var t = this._obj.responseSize; if (arguments.length) this.assert(t === v, 'expected response size to equal #{exp} but got #{act}', 'expected response size to not equal #{exp}', v, t); }, function () { utils.flag(this, 'object', this._obj.responseSize); });
      A.addMethod('jsonSchema', function (schema, opts) {
        var Ajv = loadLib('ajv'); var ajv = new Ajv(Object.assign({ logger: false, schemaId: 'auto' }, opts || {}));
        var target = isResp(this._obj) ? (function (r) { try { return r.json(); } catch (e) { return undefined; } })(this._obj) : this._obj;
        var valid = ajv.validate(schema, target);
        this.assert(valid, 'expected data to satisfy schema but found following errors: \n' + ajv.errorsText(ajv.errors), 'expected data to not satisfy schema', true, valid);
      });
      A.addMethod('cookie', function (name, value) {
        var c = this._obj.cookies; var has = c && c.has(name);
        if (arguments.length < 2) this.assert(has, "expected response to have cookie '" + name + "'", "expected response to not have cookie '" + name + "'");
        else this.assert(has && c.get(name) === value, "expected cookie '" + name + "' to have value #{exp} but got #{act}", "expected cookie '" + name + "' to not have value #{exp}", value, c && c.get(name));
      });
    });
    return chaiLib;
  }
  function defineTo(proto) {
    Object.defineProperty(proto, 'to', { configurable: true, get: function () { return getChai().expect(this).to; } });
    Object.defineProperty(proto, 'be', { configurable: true, get: function () { return getChai().expect(this).to.be; } });
  }
  Response.prototype.__pmType = 'response';
  Request.prototype.__pmType = 'request';
  defineTo(Response.prototype);
  defineTo(Request.prototype);

  // ------------------------------------------------------------ XML -> JSON (xml2js-compatible subset, explicitArray:false)
  function xml2Json(xml) {
    var pos = 0, s = String(xml);
    function decode(t) { return t.replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&quot;/g, '"').replace(/&apos;/g, "'").replace(/&#(\d+);/g, function (m, d) { return String.fromCharCode(+d); }).replace(/&#x([0-9a-f]+);/gi, function (m, h) { return String.fromCharCode(parseInt(h, 16)); }).replace(/&amp;/g, '&'); }
    function skipMisc() {
      for (;;) {
        while (pos < s.length && /\s/.test(s[pos])) pos++;
        if (s.startsWith('<?', pos)) { pos = s.indexOf('?>', pos) + 2; continue; }
        if (s.startsWith('<!--', pos)) { pos = s.indexOf('-->', pos) + 3; continue; }
        if (s.startsWith('<!DOCTYPE', pos) || s.startsWith('<!doctype', pos)) { pos = s.indexOf('>', pos) + 1; continue; }
        break;
      }
    }
    function addChild(obj, k, v) { if (obj.hasOwnProperty(k)) { if (!Array.isArray(obj[k])) obj[k] = [obj[k]]; obj[k].push(v); } else obj[k] = v; }
    function parseEl() {
      pos++; // <
      var m = /^([^\s\/>]+)/.exec(s.slice(pos)); var name = m[1]; pos += name.length;
      var attrs = null;
      for (;;) {
        while (/\s/.test(s[pos])) pos++;
        if (s[pos] === '/' || s[pos] === '>') break;
        var am = /^([^\s=\/>]+)\s*=\s*("([^"]*)"|'([^']*)')/.exec(s.slice(pos));
        if (!am) { var nm = /^([^\s=\/>]+)/.exec(s.slice(pos)); pos += nm[0].length; (attrs = attrs || {})[nm[1]] = ''; continue; }
        (attrs = attrs || {})[am[1]] = decode(am[3] !== undefined ? am[3] : am[4]); pos += am[0].length;
      }
      if (s[pos] === '/') { pos += 2; var e = attrs ? { $: attrs } : ''; return [name, e]; }
      pos++;
      var obj = {}, text = '', hasChild = false;
      for (;;) {
        if (pos >= s.length) break;
        if (s.startsWith('</', pos)) { pos = s.indexOf('>', pos) + 1; break; }
        if (s.startsWith('<!--', pos)) { pos = s.indexOf('-->', pos) + 3; continue; }
        if (s.startsWith('<![CDATA[', pos)) { var end = s.indexOf(']]>', pos); text += s.slice(pos + 9, end); pos = end + 3; continue; }
        if (s[pos] === '<') { var c = parseEl(); hasChild = true; addChild(obj, c[0], c[1]); continue; }
        var nx = s.indexOf('<', pos); if (nx < 0) nx = s.length; text += decode(s.slice(pos, nx)); pos = nx;
      }
      text = text.trim();
      if (!hasChild && !attrs) return [name, text];
      if (attrs) obj.$ = attrs;
      if (text) obj._ = text;
      return [name, obj];
    }
    skipMisc();
    if (s[pos] !== '<') return {};
    var r = parseEl(), out = {}; out[r[0]] = r[1];
    return out;
  }
  G.xml2Json = xml2Json;

  function csvParse(input, opts) {
    opts = opts || {}; var delim = opts.delimiter || ',', rows = [], row = [], f = '', q = false, s = String(input);
    for (var i = 0; i < s.length; i++) {
      var ch = s[i];
      if (q) { if (ch === '"') { if (s[i + 1] === '"') { f += '"'; i++; } else q = false; } else f += ch; }
      else if (ch === '"') q = true;
      else if (ch === delim) { row.push(f); f = ''; }
      else if (ch === '\n' || ch === '\r') { if (ch === '\r' && s[i + 1] === '\n') i++; row.push(f); f = ''; if (!(row.length === 1 && row[0] === '' && opts.skip_empty_lines)) rows.push(row); row = []; }
      else f += ch;
    }
    if (f !== '' || row.length) { row.push(f); rows.push(row); }
    if (opts.columns) { var head = rows.shift() || []; return rows.map(function (r) { var o = {}; head.forEach(function (h, i) { o[h] = r[i]; }); return o; }); }
    return rows;
  }

  // ------------------------------------------------------------ sendRequest
  function toResponse(r) {
    var res = new Response(r);
    return res;
  }
  function sendRequest(req, cb) {
    if (req && typeof req === 'object' && !(req instanceof Request) && req.body && req.body.raw !== undefined && typeof req.body.raw !== 'string') req = Object.assign({}, req, { body: Object.assign({}, req.body, { raw: JSON.stringify(req.body.raw) }) });
    var def = variables.replaceIn(exportRequest(req instanceof Request ? req : new Request(req)));
    var r = JSON.parse(__send(JSON.stringify(def)));
    var err = r.error ? new Error(r.error) : null;
    var res = err ? undefined : toResponse(r);
    if (typeof cb === 'function') {
      try { cb(err, res, { cookies: res ? res.cookies : new PList([]) }); } catch (e) { reportError(e); }
    }
    return err ? Promise.reject(err) : Promise.resolve(res);
  }

  // ------------------------------------------------------------ cookie jar
  function Jar() {}
  function jarCall(op, args, cb) {
    var r = JSON.parse(__jar(op, JSON.stringify(args)));
    if (typeof cb === 'function') { try { cb(r.error ? new Error(r.error) : null, r.result); } catch (e) { reportError(e); } }
    return r.result;
  }
  Jar.prototype.get = function (url, name, cb) { return jarCall('get', { url: String(url), name: name }, cb); };
  Jar.prototype.getAll = function (url, opts, cb) { if (typeof opts === 'function') cb = opts; return jarCall('getAll', { url: String(url) }, cb); };
  Jar.prototype.set = function (url, name, value, cb) {
    if (typeof name === 'object') { cb = value; return jarCall('set', { url: String(url), name: name.name || name.key, value: String(name.value), path: name.path, domain: name.domain, httpOnly: name.httpOnly, secure: name.secure }, cb); }
    if (typeof value === 'function') { cb = value; value = ''; }
    return jarCall('set', { url: String(url), name: name, value: String(value) }, cb);
  };
  Jar.prototype.unset = function (url, name, cb) { return jarCall('unset', { url: String(url), name: name }, cb); };
  Jar.prototype.clear = function (url, cb) { return jarCall('clear', { url: String(url) }, cb); };

  // ------------------------------------------------------------ tests
  var testIdx = 0;
  function addTest(name, fn, skipped) {
    var rec = { name: String(name), passed: true, skipped: !!skipped, error: null, index: testIdx++ };
    OUT.tests.push(rec);
    if (skipped || typeof fn !== 'function') { if (typeof fn !== 'function' && !skipped) { rec.passed = false; rec.error = 'Test function is not defined'; } return pm; }
    function fail(e) { rec.passed = false; rec.error = (e && e.name && e.name !== 'Error' ? e.name + ': ' : '') + (e && e.message !== undefined ? e.message : String(e)); }
    try {
      if (fn.length > 0) {
        // callback style: function (done) {}
        rec.pending = true;
        fn(function (err) { rec.pending = false; if (err) fail(err); });
      } else {
        var r = fn();
        if (r && typeof r.then === 'function') { rec.pending = true; r.then(function () { rec.pending = false; }, function (e) { rec.pending = false; fail(e); }); }
      }
    } catch (e) { fail(e); }
    return pm;
  }
  var testFn = function (name, fn) { return addTest(name, fn, false); };
  testFn.skip = function (name, fn) { return addTest(name, fn, true); };
  testFn.index = function () { return testIdx; };

  // ------------------------------------------------------------ pm
  var request = new Request(CTX.request);
  var response = CTX.response ? toResponse(CTX.response) : undefined;
  var reqCookies = new PList((CTX.cookies || []).map(function (c) { return { key: c.name, name: c.name, value: c.value, domain: c.domain, path: c.path }; }), { ci: false });

  var pm = {
    info: Object.freeze({ eventName: CTX.info.eventName, iteration: CTX.info.iteration, iterationCount: CTX.info.iterationCount, requestName: CTX.info.requestName, requestId: CTX.info.requestId }),
    environment: environment,
    globals: globals,
    collectionVariables: collectionVariables,
    variables: variables,
    iterationData: iterationData,
    request: request,
    response: response,
    cookies: (function () { if (response) response.cookies.each(function (c) { if (!reqCookies.has(c.key)) reqCookies.members.push(c); }); return reqCookies; })(),
    test: testFn,
    expect: function () { var c = getChai(); return c.expect.apply(c, arguments); },
    sendRequest: sendRequest,
    visualizer: { set: function (t, d, o) { OUT.visualizer = { template: String(t), data: d === undefined ? null : d, options: o || null }; }, clear: function () { OUT.visualizer = null; } },
    execution: {
      setNextRequest: function (n) { OUT.next = n === null ? null : String(n); },
      skipRequest: function () { OUT.skip = true; },
      location: Object.assign((CTX.info.path || []).slice(), { current: CTX.info.current || CTX.info.requestName }),
      runRequest: function () { return Promise.reject(new Error('pm.execution.runRequest is not supported in postmini')); }
    },
    require: function (n) { return sandboxRequire(n); },
    vault: { get: function () { return Promise.resolve(undefined); }, set: function () { return Promise.resolve(); }, unset: function () { return Promise.resolve(); } },
    message: undefined
  };
  pm.cookies.jar = function () { return new Jar(); };
  G.pm = pm;

  // ------------------------------------------------------------ legacy sandbox API
  G.tests = {};
  G.postman = {
    setEnvironmentVariable: function (k, v) { environment.set(k, v); },
    getEnvironmentVariable: function (k) { return environment.get(k); },
    clearEnvironmentVariable: function (k) { environment.unset(k); },
    clearEnvironmentVariables: function () { environment.clear(); },
    setGlobalVariable: function (k, v) { globals.set(k, v); },
    getGlobalVariable: function (k) { return globals.get(k); },
    clearGlobalVariable: function (k) { globals.unset(k); },
    clearGlobalVariables: function () { globals.clear(); },
    setNextRequest: function (n) { OUT.next = n === null ? null : String(n); },
    getResponseHeader: function (k) { return response ? response.headers.get(k) : undefined; },
    getResponseCookie: function (k) { var c = response && response.cookies.one(k); return c ? { name: c.key, value: c.value, domain: c.domain, path: c.path } : undefined; }
  };
  Object.defineProperty(G, 'environment', { get: function () { return environment.toObject(); }, configurable: true });
  Object.defineProperty(G, 'globals', { get: function () { return globals.toObject(); }, configurable: true });
  G.data = iterationData.toObject();
  G.iteration = CTX.info.iteration;
  G.request = { url: request.url.toString(), method: request.method, headers: request.getHeaders(), data: request.body && request.body.mode === 'raw' ? request.body.raw : (request.body && request.body.mode === 'urlencoded' ? request.body.urlencoded.toObject() : {}), name: CTX.info.requestName, id: CTX.info.requestId };
  if (response) {
    G.responseBody = response.text();
    G.responseCode = { code: response.code, name: response.status, detail: response.status };
    G.responseTime = response.responseTime;
    G.responseHeaders = response.headers.toObject();
    G.responseCookies = response.cookies.all().map(function (c) { return { name: c.key, value: c.value, domain: c.domain, path: c.path }; });
  }

  // ------------------------------------------------------------ finish
  G.__finish = function () {
    Object.keys(G.tests || {}).forEach(function (k) {
      OUT.tests.push({ name: k, passed: !!G.tests[k], skipped: false, error: G.tests[k] ? null : 'AssertionError: expected ' + k + ' to be truthy', index: testIdx++ });
    });
    G.tests = {};
    OUT.tests.forEach(function (t) { if (t.pending) { t.passed = false; t.error = 'Test did not complete (callback not called / promise unresolved)'; t.pending = false; } });
    var res = {
      tests: OUT.tests, console: OUT.console, errors: OUT.errors, skip: OUT.skip, visualizer: OUT.visualizer,
      vars: { locals: locals.toObject(), environment: environment.toObject(), collection: collectionVariables.toObject(), globals: globals.toObject() },
      request: exportRequest(pm.request)
    };
    if (OUT.next !== undefined) res.next = OUT.next;
    return JSON.stringify(res, function (k, v) { return v === undefined ? null : v; });
  };
};
