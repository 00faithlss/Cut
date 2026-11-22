document.addEventListener('DOMContentLoaded', () => {
  const send = (action, fields = {}) => window.ipc.postMessage(JSON.stringify({action, ...fields}));
  const status = $('.status');
  const start = document.createElement('button');
  start.className = status.className;
  start.type = 'button';
  start.textContent = 'Iniciar captura';
  status.replaceWith(start);
  const version = $('.version-line b');
  const game = $('#game');
  const monitor = $('#monitor');
  const cursor = $('#capture-cursor');
  cursor.checked = saved('captureCursor') === 'true';
  let active = false;
  let hotkeyRegistered = true;
  const source = () => option('mode') === 'Monitor completo' ? monitor : game;
  const currentProfile = () => ({
    id: source().value,
    monitor: source() === monitor,
    height: parseInt(option('resolution'), 10),
    fps: +option('fps'),
    bitrate: +option('rate'),
    seconds: +option('seconds'),
    cursor: cursor.checked,
  });
  const windowBar=document.createElement('div');
  windowBar.className='window-bar';
  const drag=document.createElement('div');
  drag.className='window-drag';
  drag.setAttribute('aria-hidden','true');
  drag.onmousedown=event=>{ if(event.button===0 && event.detail===1) send('drag'); };
  drag.ondblclick=()=>send('maximize');
  windowBar.append(drag);
  for(const [symbol,label,action] of [['−','Minimizar','minimize'],['□','Maximizar o restaurar','maximize'],['×','Cerrar ventana','hide']]){
    const button=document.createElement('button');
    button.type='button';
    button.textContent=symbol;
    button.title=label;
    button.setAttribute('aria-label',label);
    button.onclick=()=>send(action);
    windowBar.append(button);
  }
  app.prepend(windowBar);
  const savedMode=saved('nativeCaptureMode');
  profile.elements.mode.value=savedMode==='Ventana del juego'?'Ventana del juego':'Monitor completo';
  updateProfile();
  let paused=false;
  let requested='';
  let profileTimer;
  let missingSourceTimer;
  function setCaptureProfile(){
    if(paused || !source().value) return;
    const settings=currentProfile();
    const signature=JSON.stringify(settings);
    if(requested===signature) return;
    requested=signature;
    send('start',{profile:settings});
    start.textContent='Preparando replay…';
  }
  cursor.addEventListener('change',()=>{
    remember('captureCursor',String(cursor.checked));
    clearTimeout(profileTimer);
    setCaptureProfile();
  });
  function refreshCaptureChoices(){
    if(!active && !paused && !source().value) send('sources');
  }
  function changeSource(){
    const mode=option('mode');
    remember('nativeCaptureMode',mode);
    if(source().value){
      remember(mode==='Monitor completo'?'captureMonitor':'captureWindow',source().selectedOptions[0].text);
      clearTimeout(profileTimer);
      profileTimer=setTimeout(setCaptureProfile,350);
    } else {
      requested='';
      if(active) send('pause');
      send('sources');
    }
    feedback.value='';
  }
  start.onclick = () => {
    clearTimeout(profileTimer);
    paused=!paused && (active || Boolean(requested && !start.dataset.error));
    active=false;
    requested='';
    if(paused) send('pause');
    else if(source().value) setCaptureProfile();
    else {
      openPanel('clips');
      send('sources');
      feedback.value='Elige una ventana para que Cut pueda capturarla.';
    }
  };
  for (const tab of document.querySelectorAll('[data-page]')) {
    tab.addEventListener('click', () => {
      if (tab.dataset.page === 'clips') send('sources');
    });
  }
  const refreshSources = document.createElement('button');
  refreshSources.type = 'button';
  refreshSources.textContent = 'Actualizar fuentes';
  refreshSources.onclick = () => send('sources');
  $('#game-field').parentElement.append(refreshSources);
  $('#game-field').firstChild.textContent = 'Ventana ';
  $('#monitor-field').firstChild.textContent = 'Pantalla ';
  profile.addEventListener('change', changeSource);
  modePicker.addEventListener('change', changeSource);
  $('#save').onclick = () => send('save');
  document.addEventListener('keydown', event => {
    if (event.key !== 'F9' || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
    event.stopImmediatePropagation();
    event.preventDefault();
    if (!hotkeyRegistered && !event.repeat) send('save');
  }, true);
  $('#startup').addEventListener('change', event => send('startup', {enabled: event.target.checked}));
  const closePreference=$('#minimize-on-close');
  closePreference.checked=saved('minimizeOnClose')==='true';
  const syncClose=()=>{
    remember('minimizeOnClose',String(closePreference.checked));
    send('minimize_on_close',{enabled:closePreference.checked});
  };
  closePreference.addEventListener('change',syncClose);
  syncClose();

  // En escritorio el mezclador se llena con las sesiones WASAPI de Windows.
  tracks.clear();
  $('#tracks').replaceChildren();
  const microphone = $('.source');
  let nextColor = 0;
  const colors = ['#ffb57f', '#aab4ff', '#87dbaa', '#f1cf79', '#8bcfea'];
  function audioSettings(){
    return {
      device:microphone.value,
      mono:$('#mic-mono').checked,
      noise_suppression:$('#mic-noise-suppression').checked,
      tracks:Object.fromEntries([...tracks].map(([id,t])=>[id,{include:t.include.checked,volume:+t.gain.value}]))
    };
  }
  function syncAudio(){
    const settings=audioSettings();
    send('audio',{settings});
  }
  let preferences={};
  try { preferences=JSON.parse(saved('desktopAudio')) || {}; } catch {}
  function addNativeTrack(id,name,color,icon){
    addTrack([id,name,color,icon]);
    const track=tracks.get(id);
    track.level=0;
    const setting=preferences.tracks?.[id];
    if(setting){
      track.include.checked=setting.include;
      track.gain.value=setting.volume;
    }
    if(id==='mic'){
      mute.checked=!track.include.checked;
      globalGain.value=track.gain.value;
      showGain(globalGain);
      drawScale();
    }
    syncTrack(track);
    updateCount();
  }
  function addApplication(source){
    if(tracks.has(source.id)) return;
    const executable=source.label.toLowerCase();
    if(executable==='discord') addNativeTrack(source.id,'Discord','#aab4ff','discord-logo');
    else if(executable==='spotify') addNativeTrack(source.id,'Spotify','#87dbaa','spotify-logo');
    else addNativeTrack(source.id,source.label,colors[nextColor++ % colors.length],'waveform');
  }
  function applyAudioPreferences(){
    const mic=tracks.get('mic');
    mic.include.checked=preferences.tracks?.mic?.include ?? true;
    mic.gain.value=preferences.tracks?.mic?.volume ?? 100;
    mute.checked=!mic.include.checked;
    globalGain.value=mic.gain.value;
    showGain(globalGain);
    drawScale();
    syncTrack(mic);
    updateCount();
    $('#mic-mono').checked=preferences.mono ?? true;
    $('#mic-noise-suppression').checked=preferences.noise_suppression ?? true;
    microphone.replaceChildren(new Option('Micrófono predeterminado',''));
    if(preferences.device) microphone.add(new Option(preferences.device,preferences.device,true,true));
  }
  addNativeTrack('mic','Micrófono',getComputedStyle(app).getPropertyValue('--mic-tint').trim(),'microphone');
  $('#refresh').onclick=()=>send('audio_sources');
  $('#tracks').addEventListener('input',syncAudio);
  $('#tracks').addEventListener('change',syncAudio);
  $('#global-volume').addEventListener('input',syncAudio);
  $('#mic-mute').addEventListener('change',syncAudio);
  $('#mic-mono').addEventListener('change',syncAudio);
  $('#mic-noise-suppression').addEventListener('change',syncAudio);
  microphone.addEventListener('change',syncAudio);

  function fill(select, values, placeholder, preferred) {
    const previous = select.value;
    select.replaceChildren(new Option(placeholder, ''), ...values.map(item=>new Option(item.label,item.id)));
    const byLabel=values.find(item=>item.label===preferred);
    if(values.some(item=>item.id===previous)) select.value=previous;
    else if(byLabel) select.value=byLabel.id;
    else if(select===monitor && values.length) select.value=values[0].id;
  }
  window.cutDesktop = {
    toggleCapture() { start.click(); },
    ready(info) {
      version.textContent = info.version;
      $('#startup').checked = info.startup;
      hotkeyRegistered = info.hotkey;
      if (!info.hotkey) $('#save').title = 'F9 disponible con Cut en primer plano';
      if(info.restoreSettings) preferences=info.mixer.settings;
      applyAudioPreferences();
      for(const source of info.mixer.apps) addApplication(source);
      syncAudio();
    },
    sources(list) {
      fill(game,list.windows,'Elige una ventana',saved('captureWindow'));
      fill(monitor,list.monitors,'Elige una pantalla',saved('captureMonitor'));
      setCaptureProfile();
      if(!active && !paused && !source().value) start.textContent='Elige una ventana';
    },
    audioSources(catalog) {
      const label=catalog.default_microphone ? 'Predeterminado · '+catalog.default_microphone : 'Micrófono predeterminado';
      const devices=[new Option(label,''),...catalog.devices.map(d=>new Option(d.label,d.id))];
      const before=microphone.value;
      if(before && !devices.some(device=>device.value===before)){
        devices.push(new Option('Micrófono no conectado',before));
      }
      microphone.replaceChildren(...devices);
      if (devices.some(device=>device.value===before)) microphone.value=before;
      for(const source of catalog.apps) addApplication(source);
      const live=new Set(catalog.apps.map(source=>source.id));
      for(const [id,track] of tracks){
        if(id!=='mic') track.row.classList.toggle('audio-idle',!live.has(id));
      }
    },
    audioNotices(notices) {
      for(const [id, track] of tracks) {
        const message=notices[id];
        track.row.title=message || '';
        track.row.classList.toggle('audio-error',Boolean(message));
      }
    },
    audioError(message){ feedback.value='No se pudo buscar audio. '+message; },
    levels(levels) {
      for(const [id, track] of tracks){
        const level=Math.sqrt(levels[id] || 0);
        if(track.level!==level){track.level=level;drawSignal(track);}
      }
    },
    startup(enabled) { $('#startup').checked = enabled; },
    notice(message) { feedback.value = message; },
    status(state) {
      active = state.active;
      start.textContent=active?'Captura activa':!source().value?'Elige una ventana':paused?'Reanudar captura':state.error?'Reintentar captura':'Preparando replay…';
      start.title=active?'Pausar captura':start.textContent;
      if(state.error) requested='';
      start.setAttribute('aria-pressed',String(active));
      if (state.error && start.dataset.error !== state.error) feedback.value = state.error;
      start.dataset.error = state.error || '';
    }
  };
  document.documentElement.classList.add('desktop');
  send('ready');
  missingSourceTimer=setInterval(refreshCaptureChoices,3500);
  window.addEventListener('pagehide',()=>clearInterval(missingSourceTimer),{once:true});
});
