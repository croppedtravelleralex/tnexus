// grok_sign_standalone.js —— grok.com 签名器模块 1645e3 自包含执行产物 (node 直接 run)
// 模块源码：同目录 grok_sign_module_1645e3.js（live chunk 1nf91r5--cp6_.js / wrapper 4629918）
// 用法: 替换 __GROK_META__ / __SIGN_PATH__ / __SIGN_METHOD__ 后 node 执行；结果写 globalThis.__signOut

const fs = require('fs');
const vm = require('vm');
const path = require('path');
const nodeCrypto = require('crypto');

const LOG = process.argv[2] || 'access.log';
const MODULE_FILE = process.env.GROK_SIGN_MODULE || path.join(__dirname, 'grok_sign_module_1645e3.js');
const src = fs.readFileSync(MODULE_FILE, 'utf8').replace(/\s+/g, ' ').trim();

// 1645e3 模块 factory：历史上 W=> / n=> 均出现过
const modIdx = src.indexOf(',1645e3,');
if (modIdx < 0) throw new Error('module 1645e3 not found in src');
let i = src.indexOf('n=>', modIdx);
if (i < 0) i = src.indexOf('W=>', modIdx);
if (i < 0) throw new Error('module factory => not found');
const braceStart = src.indexOf('{', i);
function findMatchingBrace(s, start) {
  let depth = 0, inStr = null, esc = false;
  for (let p = start; p < s.length; p++) {
    const ch = s[p];
    if (inStr) {
      if (esc) { esc = false; continue; }
      if (ch === '\\') { esc = true; continue; }
      if (ch === inStr) inStr = null;
      continue;
    }
    if (ch === '"' || ch === "'" || ch === '`') { inStr = ch; continue; }
    if (ch === '{') depth++;
    else if (ch === '}') { depth--; if (depth === 0) return p; }
  }
  return -1;
}
const end = findMatchingBrace(src, braceStart);
const body = src.slice(i, end + 1);
// 注入：暴露内部函数用于观测（S/y/z/A 在 default 工厂内定义，故注入到 return async 之前）
const INJECT = ';globalThis.__t=t;globalThis.__n=n;globalThis.__S=S;globalThis.__y=y;globalThis.__z=z;globalThis.__A=A;';
function findReturnAsync(body) {
  for (const pat of ['return async(n,t)=>{', 'return async(W,n)=>{']) {
    const idx = body.indexOf(pat);
    if (idx >= 0) return idx;
  }
  return -1;
}
const retIdx = findReturnAsync(body);

if (retIdx < 0) throw new Error('return async not found');
let BODY = body.slice(0, retIdx) + INJECT + body.slice(retIdx);

const log = fs.createWriteStream(LOG, { flags: 'w' });
const seen = new Set();
function L(line) { if (!seen.has(line)) { seen.add(line); log.write(line + '\n'); } }

const UA = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36';

function makeLoc() {
  return { href: 'https://grok.com/', hostname: 'grok.com', host: 'grok.com', origin: 'https://grok.com', protocol: 'https:', pathname: '/', search: '', hash: '', assign() {}, replace() {}, reload() {}, toString() { return 'https://grok.com/'; } };
}

// RTCPeerConnection stub（WebRTC 指纹收集；确定性假数据）
class RTCPeerConnectionStub {
  constructor(cfg) { this.cfg = cfg; this.localDescription = null; this.connectionState = 'new'; this.iceConnectionState = 'new'; this.onicecandidate = null; this.onicegatheringstatechange = null; this.onconnectionstatechange = null; }
  createDataChannel() { return { send() {}, close() {}, readyState: 'open', onopen: null, onmessage: null }; }
  createOffer() { return Promise.resolve({ type: 'offer', sdp: 'v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\n' }); }
  createAnswer() { return Promise.resolve({ type: 'answer', sdp: 'v=0\r\n' }); }
  setLocalDescription() { return Promise.resolve(); }
  setRemoteDescription() { return Promise.resolve(); }
  addIceCandidate() { return Promise.resolve(); }
  close() {}
  getStats() { return Promise.resolve(new Map()); }
}

const getComputedStyleStub = () => {
  const base = { getPropertyValue: () => '', position: 'static', display: 'block', width: '0px', height: '0px', color: 'rgb(0, 0, 0)', cssText: '' };
  // 任意属性访问返回 '126'（UA 数字串，供签名器 x 段提取；服务端不校验 UA）
  return new Proxy(base, { get: (t, p) => (p in t ? t[p] : '126'), has: () => true });
};

const cryptoObj = {
  getRandomValues: (arr) => { for (let i = 0; i < arr.length; i++) arr[i] = Math.floor(Math.random() * 256); return arr; },
  subtle: nodeCrypto.webcrypto.subtle,
  randomUUID: () => '00000000-0000-4000-8000-000000000000',
};
const getRandomValuesCrossRealm = (arr) => {
  for (let i = 0; i < arr.length; i++) arr[i] = Math.floor(Math.random() * 256);
  return arr;
};

const stdApi = {
  Number, TextEncoder, TextDecoder, Uint8Array, Uint16Array, Uint32Array, Int8Array, Int16Array,
  Int32Array, Float32Array, Float64Array, Date, Math, Array, Promise, Function, Object, String,
  Boolean, Symbol, JSON, RegExp, Error, TypeError, RangeError, SyntaxError, Map, Set, WeakMap,
  WeakSet, Proxy, Reflect, ArrayBuffer, DataView, Blob, URL, URLSearchParams, parseInt, parseFloat,
  isNaN, isFinite, encodeURIComponent, decodeURIComponent, encodeURI, decodeURI, Uint8ClampedArray,
};

// window 顶层真实对象
const realWindow = {
  ...stdApi,
  RTCPeerConnection: RTCPeerConnectionStub,
  getComputedStyle: getComputedStyleStub,
  crypto: cryptoObj,
  btoa: (s) => Buffer.from(s, 'binary').toString('base64'),
  atob: (s) => Buffer.from(s, 'base64').toString('binary'),
  setTimeout, clearTimeout, setInterval, clearInterval,
  queueMicrotask, requestAnimationFrame: (cb) => { cb(0); return 0; },
  performance: { now: () => 0, timeOrigin: Date.now(), getEntries: () => [], mark() {}, measure() {}, timing: { navigationStart: Date.now() } },
};

function autoProxy(name, base) {
  return new Proxy(base || (() => {}), {
    get(t, p) {
      if (typeof p === 'symbol') return Reflect.get(t, p);
      if (p in t) return t[p];
      if (p === Symbol.toPrimitive) return () => '';
      if (p === Symbol.iterator) return undefined;
      if (p === 'then') return undefined;
      if (p === 'toString') return () => `[auto:${name}]`;
      if (p === 'valueOf') return () => 0;
      if (p === 'length') return 0;
      if (p === 'nodeType') return 1;
      if (p === 'nodeName') return String(name).toUpperCase();
      L(`GET ${name}.${String(p)}`);
      if (typeof p === 'string' && /^\d+$/.test(p)) return undefined;
      return autoProxy(`${name}.${String(p)}`);
    },
    set(t, p, v) { t[p] = v; return true; },
    apply() {
      L(`CALL ${name}()`);
      return autoProxy(`${name}()`);
    },
    has() { return true; },
    getPrototypeOf() { return Object.prototype; },
  });
}

// document：占位（签名器实测只解构 f=document 但不走 DOM 真值）
const emptyNodeList = [];
const docBody = autoProxy('document.body', {});
docBody.childNodes = emptyNodeList;
docBody.nodeType = 1;
const document = autoProxy('document', {});
// 真实 meta[name^=gr] content（grok-site-verification，静态站点验证值）
const grMeta = [{ name: 'grok-site-verification', content: '__GROK_META__', getAttribute: (n) => (String(n) === 'content' ? '__GROK_META__' : null), childNodes: [], parentElement: null, nodeType: 1 }];
// 假元素：childNodes[0].childNodes[1] 链 + getAttribute
function makeFakeElement() {
  const el = {
    childNodes: [
      { childNodes: [
        { getAttribute: (n) => { return null; }, textContent: '', nodeType: 1 },
      ], nodeType: 1 },
    ],
    parentElement: null,
    nodeType: 1,
    textContent: '',
    getAttribute: (n) => null,
  };
  return el;
}
document.querySelectorAll = (sel) => {
  const s = String(sel);
    if (s.startsWith('[name^=gr]')) return grMeta;
  // .r-11220 页面实测不存在（count=0）；提供不崩的 stub（z 提取走空路径）
  const leaf1 = {
    getAttribute: (n) => { return 'AAAAAAAAA11 22 33 44 55 66 77 88C99 100C2 3'; },
    textContent: '', nodeType: 1,
  };
  const child0 = { childNodes: [{ nodeType: 1, textContent: '' }, leaf1], nodeType: 1 };
  const el = { childNodes: [child0], parentElement: null, nodeType: 1, getAttribute: () => null };
  return [el, el, el, el];
};
document.querySelector = (sel) => { console.error('[QS]', JSON.stringify(String(sel))); return grMeta[0] || makeFakeElement(); };
document.body = docBody;
document.head = autoProxy('document.head', {});
document.documentElement = autoProxy('document.documentElement', {});
document.cookie = '';
document.location = makeLoc();
document.readyState = 'complete';
document.URL = 'https://grok.com/';
document.createElement = (tag) => {
  // 普通对象（无 write 等 document 方法；签名器 L() 用它做分支，必须有真实元素语义）
  return {
    style: {}, nodeType: 1, tagName: String(tag).toUpperCase(),
    childNodes: [], textContent: '', innerHTML: '', src: '', href: '',
    getAttribute: () => null, setAttribute() {}, appendChild(c) { this.childNodes.push(c); return c; },
    remove() {}, addEventListener() {}, querySelector: () => null, querySelectorAll: () => [],
    classList: { add() {}, remove() {}, contains: () => false },
    dataset: {}, parentElement: null,
  };
};
document.getElementById = () => null;
document.getElementsByTagName = () => emptyNodeList;
document.addEventListener = () => {};
document.removeEventListener = () => {};
document.write = () => {};
document.title = 'grok';

const windowObj = new Proxy(realWindow, {
  get(t, p) {
    if (typeof p === 'symbol') return Reflect.get(t, p);
    if (p in t) return t[p];
    L(`GET window.${String(p)}`);
    return autoProxy(`window.${String(p)}`);
  },
  set(t, p, v) { t[p] = v; return true; },
  has(t, p) { return true; },
});
windowObj.document = document;
windowObj.location = makeLoc();
windowObj.navigator = { userAgent: UA, platform: 'Win32', language: 'en-US', languages: ['en-US', 'en'], cookieEnabled: true, sendBeacon: () => true, webdriver: false, hardwareConcurrency: 8, maxTouchPoints: 0, plugins: [], mimeTypes: [], vendor: 'Google Inc.', deviceMemory: 8 };
windowObj.window = windowObj;
windowObj.self = windowObj;
windowObj.globalThis = windowObj;
windowObj.top = windowObj;
windowObj.parent = windowObj;
windowObj.frames = windowObj;
windowObj.innerWidth = 1920; windowObj.innerHeight = 1080; windowObj.devicePixelRatio = 1;
windowObj.screen = { width: 1920, height: 1080, availWidth: 1920, availHeight: 1080, colorDepth: 24, pixelDepth: 24 };
windowObj.history = { pushState() {}, replaceState() {}, back() {}, length: 1, state: null };
windowObj.matchMedia = () => ({ matches: false, addListener() {}, removeListener() {}, addEventListener() {}, removeEventListener() {} });
windowObj.scrollTo = () => {}; windowObj.scroll = () => {};
windowObj.open = () => null;
windowObj.alert = () => {}; windowObj.confirm = () => true; windowObj.prompt = () => null;
windowObj.localStorage = { getItem: () => null, setItem() {}, removeItem() {}, clear() {} };
windowObj.sessionStorage = { getItem: () => null, setItem() {}, removeItem() {}, clear() {} };
windowObj.XMLHttpRequest = class { open() {} send() {} setRequestHeader() {} abort() {} readyState = 0; status = 0; responseText = ''; onreadystatechange = null; };
windowObj.Headers = Map; windowObj.Request = class {}; windowObj.Response = class {};
windowObj.fetch = () => new Promise((res) => res({ ok: true, status: 200, json: () => Promise.resolve({}), text: () => Promise.resolve(''), arrayBuffer: () => Promise.resolve(new ArrayBuffer(0)), headers: new Map() }));
windowObj.crypto.getRandomValues = getRandomValuesCrossRealm;

const sandbox = {
  document, window: windowObj, navigator: windowObj.navigator, location: windowObj.location,
  performance: windowObj.performance, crypto: cryptoObj, console,
  ...stdApi, Buffer, queueMicrotask, TextEncoder, TextDecoder,
  atob: (s) => Buffer.from(s, 'base64').toString('binary'),
  btoa: (s) => Buffer.from(s, 'binary').toString('base64'),
};
sandbox.globalThis = sandbox;
vm.createContext(sandbox);

const W = { exports: {} };
W.s = (arr) => {
  if (Array.isArray(arr)) { W.exports[arr[0]] = arr[2]; }
  else { Object.assign(W.exports, arr); }
};
sandbox.__W = W;
try {
  vm.runInContext(`(${BODY})(__W)`, sandbox, { timeout: 15000 });
} catch (e) {
  console.error('MODULE EXEC ERROR:', e.message);
  log.end(); process.exit(2);
}
const factory = W.exports.default;
if (typeof factory !== 'function') { console.error('NO DEFAULT FACTORY; exports keys:', Object.keys(W.exports)); log.end(); process.exit(3); }
let signer;
try { signer = factory(); } catch (e) { console.error('FACTORY ERROR:', e.message); if (e.stack) console.error(e.stack.split('\n').slice(0, 8).join('\n')); log.end(); process.exit(4); }
if (typeof signer !== 'function') { console.error('FACTORY DID NOT RETURN SIGNER:', typeof signer); log.end(); process.exit(5); }
globalThis.__signOut = signer('__SIGN_PATH__', '__SIGN_METHOD__');
if (typeof Promise !== 'undefined') {
  Promise.resolve(globalThis.__signOut).then(function (v) {
    var sv = String(v);
    console.log('FULLSIG', sv.length, sv);
  }).catch(function (e) {
    console.error('SIGN ERR', e && (e.stack || String(e)));
  });
}
