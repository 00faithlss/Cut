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
  const defaultHotkey = {key:120,modifiers:0};
  let clipHotkey = defaultHotkey;
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
  let restoredSource;
  let requested='';
  let profileTimer;
  let missingSourceTimer;
  function setCaptureProfile(){
    if(paused){
      send('select_paused_profile',{profile:source().value?currentProfile():null});
      return;
    }
    if(!source().value) return;
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
      remember('activePanel',tab.dataset.page);
      if (tab.dataset.page === 'clips') send('sources');
    });
  }
  $('#motion').addEventListener('click',()=>remember('backgroundMotion',String(swimming)));
  const refreshSources = document.createElement('button');
  refreshSources.type = 'button';
  refreshSources.textContent = 'Actualizar fuentes';
  refreshSources.onclick = () => send('sources');
  $('#game-field').parentElement.append(refreshSources);
  $('#game-field').firstChild.textContent = 'Ventana ';
  $('#monitor-field').firstChild.textContent = 'Pantalla ';
  profile.addEventListener('change', changeSource);
  modePicker.addEventListener('change', changeSource);
  $('#open-directory').onclick = () => send('open_directory');
  $('#choose-directory').onclick = () => send('choose_directory');
  $('#save').onclick = () => send('save');
  const updateButton = $('#update-available');
  let updateInstalling=false;
  let updateVersion='';
  let updateProgress=null;
  function renderUpdate(){
    updateButton.hidden=!updateVersion;
    updateButton.disabled=updateInstalling || updateProgress!==null;
    updateButton.textContent=updateInstalling?'Instalando · Cut se reiniciará':
      updateProgress!==null?'Descargando actualización · '+updateProgress+' %':
      'Actualización disponible · Descargar versión '+updateVersion;
    updateButton.title='Se descargará desde Cut y la app se reiniciará automáticamente al terminar.';
  }
  updateButton.onclick=()=>send('open_update');
  $('#check-updates').onclick=()=>send('check_updates');
  $('#cut-updates').hidden=false;
  const hotkeySection = $('#clip-hotkey');
  const hotkeyKey = $('#hotkey-key');
  const hotkeyValue = $('#hotkey-value');
  const hotkeyReset = $('#hotkey-reset');
  const hotkeyFeedback = $('#hotkey-feedback');
  let editingHotkey = false;
  let armedHotkey = false;
  let pendingHotkey;
  const numpadKeys = {NumpadEnter:14,NumpadMultiply:106,NumpadAdd:107,NumpadSubtract:109,NumpadDecimal:110,NumpadDivide:111};
  const numpadLabels = {14:'Num Enter',106:'Num ×',107:'Num +',109:'Num −',110:'Num .',111:'Num /'};
  const isNumpad = key => (key>=96 && key<=105) || Object.hasOwn(numpadLabels,key);
  const keyLabel = key => numpadLabels[key] || (key>=96 && key<=105 ? 'Num '+(key-96) : key >= 112 ? 'F'+(key-111) : String.fromCharCode(key));
  const modifierNames = [[1,'Ctrl'],[2,'Alt'],[4,'Mayús']];
  function keyCode(event){
    if(/^Numpad[0-9]$/.test(event.code)) return 96+Number(event.code.slice(6));
    if(Object.hasOwn(numpadKeys,event.code)) return numpadKeys[event.code];
    if(/^F(?:[1-9]|1[0-9]|2[0-4])$/.test(event.key)) return 111+Number(event.key.slice(1));
    if(/^Digit[0-9]$/.test(event.code)) return event.code.charCodeAt(5);
    if(/^[A-Za-z0-9]$/.test(event.key)) return event.key.toUpperCase().charCodeAt(0);
  }
  function showHotkey(binding){
    clipHotkey=binding;
    const modifiers=modifierNames.filter(([flag])=>binding.modifiers&flag).map(([,name])=>name);
    const label=[...modifiers,keyLabel(binding.key)].join(' + ');
    hotkeyValue.textContent=editingHotkey?'…':label;
    hotkeyKey.setAttribute('aria-pressed',String(editingHotkey));
    hotkeyReset.hidden=editingHotkey || (binding.key===defaultHotkey.key && !binding.modifiers);
    $('#save kbd').textContent=label;
    $('#save').title=hotkeyRegistered ? 'Guardar clip · '+label : label+' disponible con Cut en primer plano';
    $('#clip-duration-legend').textContent='Tiempo que guarda '+label;
  }
  function setHotkeyEditing(editing){
    editingHotkey=editing;
    armedHotkey=false;
    pendingHotkey=undefined;
    hotkeyFeedback.value=editing?'Pulsa una tecla. Esc cancela.':'';
    showHotkey(clipHotkey);
    send('hotkey_edit',{editing});
  }
  hotkeyKey.onclick=()=>setHotkeyEditing(!editingHotkey);
  hotkeyKey.onblur=()=>{
    if(editingHotkey) setHotkeyEditing(false);
  };
  window.addEventListener('blur',()=>{
    if(editingHotkey) setHotkeyEditing(false);
  });
  hotkeyReset.onclick=()=>{
    hotkeyKey.focus();
    send('hotkey',{binding:defaultHotkey});
  };
  hotkeySection.hidden=false;
  document.addEventListener('keydown', event => {
    const modifiers=Number(event.ctrlKey)+2*Number(event.altKey)+4*Number(event.shiftKey);
    const key=keyCode(event);
    if(editingHotkey){
      if(event.key==='Tab'){
        setHotkeyEditing(false);
        return;
      }
      event.stopImmediatePropagation();
      event.preventDefault();
      if(event.key==='Escape'){
        setHotkeyEditing(false);
        return;
      }
      if(!armedHotkey || pendingHotkey || event.repeat || event.isComposing ||
          ['Control','Alt','Shift','Meta','AltGraph'].includes(event.key)) return;
      if(!key || event.metaKey || (key<112 && !isNumpad(key) && !(modifiers&3))){
        hotkeyFeedback.value='Usa F1–F24, una tecla del numpad o Ctrl/Alt con una letra o número.';
        return;
      }
      pendingHotkey={key,modifiers,code:event.code || event.key};
      return;
    }
    if(event.metaKey || modifiers!==clipHotkey.modifiers || key!==clipHotkey.key) return;
    event.stopImmediatePropagation();
    event.preventDefault();
    if (!hotkeyRegistered && !event.repeat) send('save');
  }, true);
  document.addEventListener('keyup', event => {
    if(!editingHotkey || pendingHotkey?.code!==(event.code || event.key)) return;
    event.preventDefault();
    const {key,modifiers}=pendingHotkey;
    pendingHotkey=undefined;
    armedHotkey=false;
    send('hotkey',{binding:{key,modifiers}});
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
  const clipAudio = $('#clip-audio');
  let nextColor = 0;
  const colors = ['#ffb57f', '#aab4ff', '#87dbaa', '#f1cf79', '#8bcfea'];
  function audioSettings(){
    return {
      device:microphone.value,
      mono:$('#mic-mono').checked,
      noise_suppression:$('#mic-noise-suppression').checked,
      clip_audio:clipAudio.value,
      push_to_talk:preferences.push_to_talk ?? null,
      tracks:{
        ...preferences.tracks,
        ...Object.fromEntries([...tracks].map(([id,t])=>[id,{include:t.include.checked,volume:+t.gain.value}]))
      }
    };
  }
  function syncAudio(){
    preferences=audioSettings();
    send('audio',{settings:preferences});
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
    clipAudio.value=preferences.clip_audio==='mixed'?'mixed':'separate';
    microphone.replaceChildren(new Option('Micrófono predeterminado',''));
    if(preferences.device) microphone.add(new Option(preferences.device,preferences.device,true,true));
  }
  addNativeTrack('mic','Micrófono',getComputedStyle(app).getPropertyValue('--mic-tint').trim(),'microphone');
  tracks.get('mic').row.hidden=true;
  let lastMicSignal=-Infinity;
  updateCount();
  $('#refresh').onclick=()=>send('audio_sources');
  $('#tracks').addEventListener('input',syncAudio);
  $('#tracks').addEventListener('change',syncAudio);
  $('#global-volume').addEventListener('input',syncAudio);
  $('#mic-mute').addEventListener('change',syncAudio);
  $('#mic-mono').addEventListener('change',syncAudio);
  $('#mic-noise-suppression').addEventListener('change',syncAudio);
  microphone.addEventListener('change',syncAudio);
  clipAudio.addEventListener('change',event=>{
    event.stopPropagation();
    syncAudio();
  });
  $('#clip-audio-field').hidden=false;

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
    directory(path) { $('#clip-directory').textContent = path; },
    ready(info) {
      paused=info.paused;
      restoredSource=info.sourceId;
      const panel=saved('activePanel');
      if(panelNames.includes(panel)) openPanel(panel);
      if(saved('backgroundMotion')!==null){
        swimming=saved('backgroundMotion')==='true';
        syncMotion();
      }
      window.cutDesktop.directory(info.directory);
      version.textContent = info.version;
      $('#startup').checked = info.startup;
      hotkeyRegistered = info.hotkey;
      showHotkey(info.clipHotkey);
      if(info.restoreSettings) preferences=info.audioSettings;
      applyAudioPreferences();
      syncAudio();
    },
    sources(list) {
      fill(game,list.windows,'Elige una ventana',saved('captureWindow'));
      fill(monitor,list.monitors,'Elige una pantalla',saved('captureMonitor'));
      if(restoredSource && [...source().options].some(item=>item.value===restoredSource)) source().value=restoredSource;
      restoredSource=null;
      if(!paused && !source().value) {
        const key=source()===monitor?'captureMonitor':'captureWindow';
        send('await_source',{profile:currentProfile(),label:saved(key)});
      } else {
        setCaptureProfile();
      }
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
    },
    audible(sources) {
      const live=new Set(sources.map(source=>source.id));
      for(const [id,track] of tracks){
        if(id!=='mic' && !live.has(id)){
          track.row.remove();
          tracks.delete(id);
        }
      }
      for(const source of sources) addApplication(source);
      updateCount();
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
      const mic=tracks.get('mic');
      const now=performance.now();
      if(mic.include.checked && levels.mic >= 0.001) lastMicSignal=now;
      const hideMic=!active || !mic.include.checked || now-lastMicSignal >= 3000;
      if(mic.row.hidden!==hideMic){
        mic.row.hidden=hideMic;
        updateCount();
      }
      for(const [id, track] of tracks){
        if(track.row.hidden) continue;
        const level=Math.sqrt(levels[id] || 0);
        if(track.level!==level){track.level=level;drawSignal(track);}
      }
    },
    startup(enabled) { $('#startup').checked = enabled; },
    updateAvailable(update) {
      updateVersion=update?.version || '';
      renderUpdate();
    },
    updateProgress(percent) { updateProgress=percent; renderUpdate(); },
    updateInstalling(installing) { updateInstalling=installing; renderUpdate(); },
    updateMessage(message) { $('#update-message').value=message; },
    hotkey(binding) {
      editingHotkey=false;
      armedHotkey=false;
      showHotkey(binding);
      hotkeyFeedback.value='';
    },
    hotkeyEditing(editing) { armedHotkey=editing && editingHotkey; },
    hotkeyError(message) {
      armedHotkey=editingHotkey;
      hotkeyFeedback.value=message;
    },
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
