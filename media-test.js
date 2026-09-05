// Real end-to-end WebRTC media test.
// Publishes a fake camera via WHIP through the public HTTPS tunnel, then the
// caller checks MediaMTX's byte counters to prove RTP actually transited -
// signalling success alone proves nothing.
const { chromium } = require('@playwright/test');

(async () => {
  const [publishUrl, token] = process.argv.slice(2);
  if (!publishUrl || !token) { console.log('RESULT usage-error'); process.exit(2); }

  const browser = await chromium.launch({
    args: [
      '--use-fake-device-for-media-stream',   // synthetic camera+mic, no hardware
      '--use-fake-ui-for-media-stream',       // auto-accept permission prompt
      '--allow-running-insecure-content',
      '--autoplay-policy=no-user-gesture-required',
    ],
  });
  const page = await browser.newPage();
  page.on('console', m => console.log('  [browser]', m.text()));

  // about:blank has no origin for fetch; serve from the live app so the WHIP
  // POST is same-site and CSP/CORS behave as they do for a real user.
  await page.goto('https://attend-managed-advertisements-assigned.trycloudflare.com/', { waitUntil: 'domcontentloaded' });

  const result = await page.evaluate(async ([url, tok]) => {
    const log = [];
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ video: true, audio: true });
      log.push('getUserMedia ok: ' + stream.getTracks().map(t => t.kind).join('+'));

      const pc = new RTCPeerConnection({
        iceServers: [
          { urls: 'stun:stun.l.google.com:19302' },
          { urls: 'turn:openrelay.metered.ca:443?transport=tcp',
            username: 'openrelayproject', credential: 'openrelayproject' },
        ],
      });
      stream.getTracks().forEach(t => pc.addTrack(t, stream));

      const offer = await pc.createOffer();
      await pc.setLocalDescription(offer);
      // Wait for ICE gathering so the offer carries candidates.
      await new Promise(res => {
        if (pc.iceGatheringState === 'complete') return res();
        const t = setTimeout(res, 8000);
        pc.addEventListener('icegatheringstatechange', () => {
          if (pc.iceGatheringState === 'complete') { clearTimeout(t); res(); }
        });
      });

      const cands = (pc.localDescription.sdp.match(/a=candidate:/g) || []).length;
      const relay = (pc.localDescription.sdp.match(/typ relay/g) || []).length;
      log.push(`local candidates=${cands} relay=${relay}`);

      const resp = await fetch(url, {
        method: 'POST',
        headers: { 'Content-Type': 'application/sdp', 'Authorization': 'Basic ' + btoa(':' + tok) },
        body: pc.localDescription.sdp,
      });
      log.push('WHIP POST status=' + resp.status);
      if (!resp.ok) return { ok: false, log, body: (await resp.text()).slice(0, 200) };

      await pc.setRemoteDescription({ type: 'answer', sdp: await resp.text() });
      log.push('remote description set');

      // Let ICE connect and media flow.
      const deadline = Date.now() + 25000;
      while (Date.now() < deadline) {
        if (['connected', 'completed'].includes(pc.iceConnectionState)) break;
        if (pc.iceConnectionState === 'failed') break;
        await new Promise(r => setTimeout(r, 500));
      }
      log.push('iceConnectionState=' + pc.iceConnectionState);

      await new Promise(r => setTimeout(r, 90000)); // hold the publish so a viewer can attach

      let sent = 0, pairType = '';
      const stats = await pc.getStats();
      stats.forEach(s => {
        if (s.type === 'outbound-rtp') sent += (s.bytesSent || 0);
        if (s.type === 'candidate-pair' && s.state === 'succeeded') pairType = s.remoteCandidateId || 'succeeded';
      });
      log.push(`outbound bytesSent=${sent} selectedPair=${pairType || 'none'}`);
      return { ok: true, log, ice: pc.iceConnectionState, bytesSent: sent };
    } catch (e) {
      log.push('EXCEPTION ' + e.message);
      return { ok: false, log };
    }
  }, [publishUrl, token]);

  result.log.forEach(l => console.log('  ' + l));
  console.log('RESULT ' + JSON.stringify({ ok: result.ok, ice: result.ice, bytesSent: result.bytesSent }));
  await browser.close();
})();
