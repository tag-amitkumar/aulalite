#!/usr/bin/env node
// Real Teacher -> Student WebRTC test over the PUBLIC production path.
//
// Replaces the earlier media-test.js / watch-test.js pair, both of which had a
// dead `trycloudflare.com` origin baked into the source.
//
// What this proves that a signalling check does NOT:
//   * the SDP exchange reaches MediaMTX through the public media hostname,
//   * MediaMTX's CORS policy accepts the app origin (a cross-origin WHIP/WHEP
//     POST from the app to the media host is what a real browser does),
//   * RTP actually transits, measured as inbound framesDecoded on the STUDENT.
//
// --relay-only forces `iceTransportPolicy: 'relay'` on both peers, so every
// host and server-reflexive candidate is discarded and the only way media can
// flow is through TURN. That is a STRICTLY HARDER condition than teacher and
// student sitting on different networks, so it is the honest way to prove the
// cross-network path from a single machine.
//
// Teacher and student run in two separate Chromium PROCESSES with separate
// profiles, so they share no ICE state, no DNS cache and no session storage.
//
// Usage:
//   node ops/mediamtx/e2e-webrtc-test.js \
//     --app https://aula.elementors.guru \
//     --api https://aula.elementors.guru \
//     --course <course-uuid> \
//     --teacher-token <tok> --student-token <tok> [--relay-only]

const { chromium } = require('@playwright/test');

function arg(name, dflt) {
  const i = process.argv.indexOf(`--${name}`);
  return i > -1 ? process.argv[i + 1] : dflt;
}
const FLAG = (name) => process.argv.includes(`--${name}`);

const APP = (arg('app', 'https://aula.elementors.guru') || '').replace(/\/$/, '');
const API = (arg('api', APP) || '').replace(/\/$/, '');
const COURSE = arg('course');
const TEACHER = arg('teacher-token');
const STUDENT = arg('student-token');
const RELAY_ONLY = FLAG('relay-only');
// --media-host lets the harness retarget the media leg at a different origin
// than the backend advertised. Used ONLY to exercise the loopback media plane
// while public DNS for the media hostname is not yet in place; a production
// run must never pass it (and --require-public-media asserts that).
const MEDIA_HOST = arg('media-host');
// Origin the test pages load from. Defaults to the app, which is what makes the
// WHIP/WHEP POST exercise the production CSP and MediaMTX CORS. Override only
// for a loopback media-plane test, where the app CSP's `https:` connect-src
// would reject a plain-http media origin.
const PAGE = (arg('page', APP) || '').replace(/\/$/, '');
const REQUIRE_PUBLIC = FLAG('require-public-media');
const retarget = (u) => {
  if (!u || !MEDIA_HOST) return u;
  const src = new URL(u), dst = new URL(MEDIA_HOST);
  src.protocol = dst.protocol; src.host = dst.host;
  return src.toString();
};

const api = async (method, path, token, body) => {
  const r = await fetch(`${API}${path}`, {
    method,
    headers: {
      Authorization: `Bearer ${token}`,
      ...(body ? { 'Content-Type': 'application/json' } : {}),
    },
    ...(body ? { body: JSON.stringify(body) } : {}),
  });
  const text = await r.text();
  let json = null;
  try { json = JSON.parse(text); } catch { /* non-JSON error body */ }
  if (!r.ok) throw new Error(`${method} ${path} -> ${r.status} ${text.slice(0, 300)}`);
  return json;
};

// ---------------------------------------------------------------------------
// Browser-side halves. These run inside the page so they use the browser's own
// WebRTC stack and the page's origin (which is what makes the WHIP/WHEP POST a
// genuine cross-origin request subject to MediaMTX's CORS policy).
// ---------------------------------------------------------------------------

const publishInPage = async ([url, password, iceServers, relayOnly]) => {
  const log = [];
  const cfg = { iceServers };
  if (relayOnly) cfg.iceTransportPolicy = 'relay';
  const pc = new RTCPeerConnection(cfg);
  const iceErrors = [];
  pc.addEventListener('icecandidateerror', (e) =>
    iceErrors.push(`code=${e.errorCode} ${e.errorText} url=${e.url}`));
  try {
    const stream = await navigator.mediaDevices.getUserMedia({ video: true, audio: true });
    log.push(`getUserMedia ok: ${stream.getTracks().map((t) => t.kind).join('+')}`);
    stream.getTracks().forEach((t) => pc.addTrack(t, stream));

    await pc.setLocalDescription(await pc.createOffer());
    await new Promise((res) => {
      if (pc.iceGatheringState === 'complete') return res();
      const t = setTimeout(res, 15000);
      pc.addEventListener('icegatheringstatechange', () => {
        if (pc.iceGatheringState === 'complete') { clearTimeout(t); res(); }
      });
    });
    const sdp = pc.localDescription.sdp;
    const types = {};
    for (const m of sdp.matchAll(/ typ (\w+)/g)) types[m[1]] = (types[m[1]] || 0) + 1;
    log.push(`offer candidates: ${JSON.stringify(types)}`);
    if (relayOnly && !types.relay) {
      return { ok: false, log, err: 'relay-only requested but TURN produced no relay candidate', iceErrors };
    }

    const resp = await fetch(url, {
      method: 'POST',
      headers: { 'Content-Type': 'application/sdp', Authorization: 'Basic ' + btoa(':' + password) },
      body: sdp,
    });
    log.push(`WHIP POST ${url} -> ${resp.status}`);
    if (!resp.ok) return { ok: false, log, err: (await resp.text()).slice(0, 300), iceErrors };
    await pc.setRemoteDescription({ type: 'answer', sdp: await resp.text() });

    const deadline = Date.now() + 30000;
    while (Date.now() < deadline) {
      if (['connected', 'completed'].includes(pc.iceConnectionState)) break;
      if (pc.iceConnectionState === 'failed') break;
      await new Promise((r) => setTimeout(r, 300));
    }
    log.push(`iceConnectionState=${pc.iceConnectionState}`);
    window.__pc = pc; // keep alive for the later stats read
    return { ok: pc.iceConnectionState === 'connected' || pc.iceConnectionState === 'completed', log, iceErrors };
  } catch (e) {
    log.push('EXCEPTION ' + e.message);
    return { ok: false, log, iceErrors };
  }
};

const subscribeInPage = async ([url, jwt, iceServers, relayOnly]) => {
  const log = [];
  const cfg = { iceServers };
  if (relayOnly) cfg.iceTransportPolicy = 'relay';
  const pc = new RTCPeerConnection(cfg);
  const iceErrors = [];
  pc.addEventListener('icecandidateerror', (e) =>
    iceErrors.push(`code=${e.errorCode} ${e.errorText} url=${e.url}`));
  try {
    // WHEP is receive-only: the offer must declare recvonly transceivers.
    pc.addTransceiver('video', { direction: 'recvonly' });
    pc.addTransceiver('audio', { direction: 'recvonly' });
    let tracks = 0;
    pc.addEventListener('track', () => { tracks++; });

    await pc.setLocalDescription(await pc.createOffer());
    await new Promise((res) => {
      if (pc.iceGatheringState === 'complete') return res();
      const t = setTimeout(res, 15000);
      pc.addEventListener('icegatheringstatechange', () => {
        if (pc.iceGatheringState === 'complete') { clearTimeout(t); res(); }
      });
    });
    const types = {};
    for (const m of pc.localDescription.sdp.matchAll(/ typ (\w+)/g)) types[m[1]] = (types[m[1]] || 0) + 1;
    log.push(`offer candidates: ${JSON.stringify(types)}`);
    if (relayOnly && !types.relay) {
      return { ok: false, log, err: 'relay-only requested but TURN produced no relay candidate', iceErrors };
    }

    const resp = await fetch(url, {
      method: 'POST',
      headers: { 'Content-Type': 'application/sdp', Authorization: 'Basic ' + btoa(':' + jwt) },
      body: pc.localDescription.sdp,
    });
    log.push(`WHEP POST ${url} -> ${resp.status}`);
    if (!resp.ok) return { ok: false, log, err: (await resp.text()).slice(0, 300), iceErrors };
    await pc.setRemoteDescription({ type: 'answer', sdp: await resp.text() });

    const deadline = Date.now() + 30000;
    while (Date.now() < deadline) {
      if (['connected', 'completed'].includes(pc.iceConnectionState)) break;
      if (pc.iceConnectionState === 'failed') break;
      await new Promise((r) => setTimeout(r, 300));
    }
    log.push(`iceConnectionState=${pc.iceConnectionState}`);

    // Let real media arrive. framesDecoded is the only honest proof.
    await new Promise((r) => setTimeout(r, 12000));

    let bytes = 0, packets = 0, frames = 0, pairs = [];
    const stats = await pc.getStats();
    const byId = new Map();
    stats.forEach((s) => byId.set(s.id, s));
    stats.forEach((s) => {
      if (s.type === 'inbound-rtp') {
        bytes += s.bytesReceived || 0;
        packets += s.packetsReceived || 0;
        frames += s.framesDecoded || 0;
      }
      if (s.type === 'candidate-pair' && s.state === 'succeeded' && s.nominated) {
        const l = byId.get(s.localCandidateId), r = byId.get(s.remoteCandidateId);
        pairs.push(`${l ? l.candidateType : '?'}(${l ? l.protocol : '?'}) -> ${r ? r.candidateType : '?'}`);
      }
    });
    log.push(`tracks=${tracks} bytesReceived=${bytes} packets=${packets} framesDecoded=${frames}`);
    log.push(`selected pair: ${pairs.join(', ') || 'none'}`);
    return { ok: frames > 0 && bytes > 0, log, frames, bytes, packets, pairs, iceErrors };
  } catch (e) {
    log.push('EXCEPTION ' + e.message);
    return { ok: false, log, iceErrors };
  }
};

const LAUNCH = {
  args: [
    '--use-fake-device-for-media-stream', // synthetic camera+mic, no hardware
    '--use-fake-ui-for-media-stream',     // auto-accept the permission prompt
    '--autoplay-policy=no-user-gesture-required',
  ],
};

(async () => {
  if (!COURSE || !TEACHER || !STUDENT) {
    console.error('need --course, --teacher-token and --student-token');
    process.exit(2);
  }
  console.log(`app=${APP}  relayOnly=${RELAY_ONLY}`);

  // 1. Teacher starts a class and goes live. A course allows only one live
  // session, so clear a leftover one from an aborted run first — otherwise
  // start-now answers 409 and the harness is unusable until someone ends the
  // class by hand.
  let started;
  try {
    started = await api('POST', `/v1/courses/${COURSE}/sessions/start-now`, TEACHER, {});
  } catch (e) {
    const stale = /"active_session_id":"([0-9a-f-]+)"/.exec(e.message);
    if (!stale) throw e;
    console.log(`ending stale live session ${stale[1]}`);
    await api('POST', `/v1/sessions/${stale[1]}/end-class`, TEACHER, {});
    started = await api('POST', `/v1/courses/${COURSE}/sessions/start-now`, TEACHER, {});
  }
  const sessionId = started.id || started.session_id;
  console.log(`session=${sessionId}`);
  const golive = await api('POST', `/v1/sessions/${sessionId}/go-live`, TEACHER, {});
  console.log(`publish url = ${golive.main_publish_url}`);
  console.log(`teacher ice = ${JSON.stringify(golive.ice_servers)}`);

  // 2. Student joins and gets the WHEP url + viewer jwt.
  const join = await api('POST', `/v1/sessions/${sessionId}/join`, STUDENT, {});
  console.log(`whep url = ${join.main_url}`);
  console.log(`student ice = ${JSON.stringify(join.ice_servers)}`);

  // The app must never hand out a loopback media host: that is the exact
  // production defect this path regressed on before. Checked against what the
  // BACKEND returned, before any --media-host retargeting.
  for (const [label, u] of [['publish', golive.main_publish_url], ['whep', join.main_url]]) {
    if (/localhost|127\.0\.0\.1/.test(u || '')) {
      console.log(`RESULT FAIL ${label} url is loopback: ${u}`);
      process.exit(1);
    }
  }
  const publishUrl = retarget(golive.main_publish_url);
  const whepUrl = retarget(join.main_url);
  if (MEDIA_HOST) {
    if (REQUIRE_PUBLIC) {
      console.log('RESULT FAIL --media-host passed together with --require-public-media');
      process.exit(1);
    }
    console.log(`NOTE media leg retargeted to ${MEDIA_HOST} (loopback plane test, NOT the public path)`);
  }

  // 3. Two independent browser processes: teacher publishes, student views.
  const teacherBrowser = await chromium.launch(LAUNCH);
  const studentBrowser = await chromium.launch(LAUNCH);
  try {
    const tPage = await teacherBrowser.newPage();
    tPage.on('console', (m) => console.log('  [teacher console]', m.text()));
    tPage.on('pageerror', (e) => console.log('  [teacher pageerror]', e.message));
    await tPage.goto(`${PAGE}/`, { waitUntil: 'domcontentloaded' });

    const pub = await tPage.evaluate(publishInPage,
      [publishUrl, golive.publish_password, golive.ice_servers || [], RELAY_ONLY]);
    console.log('--- TEACHER ---');
    pub.log.forEach((l) => console.log('  ' + l));
    (pub.iceErrors || []).slice(0, 5).forEach((l) => console.log('  ice-error: ' + l));
    if (pub.err) console.log('  err: ' + pub.err);
    // Throw rather than process.exit: exit() skips the finally block, which
    // would leave the class LIVE and make the next run fail with 409.
    if (!pub.ok) { console.log('RESULT FAIL teacher-publish'); throw new Error('teacher-publish'); }

    const sPage = await studentBrowser.newPage();
    sPage.on('console', (m) => console.log('  [student console]', m.text()));
    sPage.on('pageerror', (e) => console.log('  [student pageerror]', e.message));
    await sPage.goto(`${PAGE}/`, { waitUntil: 'domcontentloaded' });

    const sub = await sPage.evaluate(subscribeInPage,
      [whepUrl, join.viewer_jwt, join.ice_servers || [], RELAY_ONLY]);
    console.log('--- STUDENT ---');
    sub.log.forEach((l) => console.log('  ' + l));
    (sub.iceErrors || []).slice(0, 5).forEach((l) => console.log('  ice-error: ' + l));
    if (sub.err) console.log('  err: ' + sub.err);

    // Teacher-side outbound counters, read after the student attached.
    const tstats = await tPage.evaluate(async () => {
      let sent = 0;
      (await window.__pc.getStats()).forEach((s) => {
        if (s.type === 'outbound-rtp') sent += s.bytesSent || 0;
      });
      return { sent, ice: window.__pc.iceConnectionState };
    });
    console.log(`  teacher outbound bytesSent=${tstats.sent} ice=${tstats.ice}`);

    console.log('RESULT ' + JSON.stringify({
      ok: sub.ok,
      relayOnly: RELAY_ONLY,
      framesDecoded: sub.frames || 0,
      bytesReceived: sub.bytes || 0,
      teacherBytesSent: tstats.sent,
      selectedPair: sub.pairs || [],
    }));
    // Set the code instead of exiting, so `finally` still ends the class.
    if (!sub.ok) process.exitCode = 1;
  } finally {
    await teacherBrowser.close();
    await studentBrowser.close();
    try { await api('POST', `/v1/sessions/${sessionId}/end-class`, TEACHER, {}); } catch { /* best effort */ }
  }
})().catch((e) => { console.error('FATAL ' + e.message); process.exit(1); });
