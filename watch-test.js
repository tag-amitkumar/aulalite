// Student side: subscribe via WHEP and measure INBOUND RTP.
const { chromium } = require('@playwright/test');
(async () => {
  const [whepUrl, jwt] = process.argv.slice(2);
  const browser = await chromium.launch({ args: ['--use-fake-device-for-media-stream','--use-fake-ui-for-media-stream','--autoplay-policy=no-user-gesture-required'] });
  const page = await browser.newPage();
  await page.goto('https://attend-managed-advertisements-assigned.trycloudflare.com/', { waitUntil: 'domcontentloaded' });
  const res = await page.evaluate(async ([url, tok]) => {
    const log = [];
    try {
      const pc = new RTCPeerConnection({ iceServers: [
        { urls: 'stun:stun.l.google.com:19302' },
        { urls: 'turn:openrelay.metered.ca:443?transport=tcp', username: 'openrelayproject', credential: 'openrelayproject' },
      ]});
      // WHEP is receive-only: declare recvonly transceivers before the offer.
      pc.addTransceiver('video', { direction: 'recvonly' });
      pc.addTransceiver('audio', { direction: 'recvonly' });
      let gotTrack = 0;
      pc.addEventListener('track', () => { gotTrack++; });
      const offer = await pc.createOffer();
      await pc.setLocalDescription(offer);
      await new Promise(r => { if (pc.iceGatheringState==='complete') return r(); const t=setTimeout(r,8000);
        pc.addEventListener('icegatheringstatechange',()=>{ if(pc.iceGatheringState==='complete'){clearTimeout(t);r();} }); });
      const resp = await fetch(url, { method:'POST', headers:{ 'Content-Type':'application/sdp', 'Authorization':'Basic '+btoa(':'+tok) }, body: pc.localDescription.sdp });
      log.push('WHEP POST status=' + resp.status);
      if (!resp.ok) return { ok:false, log, body:(await resp.text()).slice(0,200) };
      await pc.setRemoteDescription({ type:'answer', sdp: await resp.text() });
      const deadline = Date.now()+25000;
      while (Date.now()<deadline) { if (['connected','completed'].includes(pc.iceConnectionState)) break;
        if (pc.iceConnectionState==='failed') break; await new Promise(r=>setTimeout(r,500)); }
      log.push('iceConnectionState=' + pc.iceConnectionState);
      await new Promise(r=>setTimeout(r,7000));
      let recv=0, packets=0;
      (await pc.getStats()).forEach(s => { if (s.type==='inbound-rtp') { recv += (s.bytesReceived||0); packets += (s.packetsReceived||0); } });
      log.push(`tracks=${gotTrack} inbound bytesReceived=${recv} packets=${packets}`);
      return { ok:true, log, ice:pc.iceConnectionState, bytesReceived:recv, packets };
    } catch(e) { log.push('EXCEPTION '+e.message); return { ok:false, log }; }
  }, [whepUrl, jwt]);
  res.log.forEach(l=>console.log('  '+l));
  console.log('RESULT ' + JSON.stringify({ ok:res.ok, ice:res.ice, bytesReceived:res.bytesReceived, packets:res.packets }));
  await browser.close();
})();
