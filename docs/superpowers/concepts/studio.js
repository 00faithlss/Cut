const profile = $('#clip-profile');
const option = name => profile.elements.namedItem(name).value;
const modePicker = $('.source-picker select');
function updateProfile() {
  const monitor = option('mode') === 'Monitor completo';
  $('#monitor-field').hidden = !monitor;
  $('#game-field').hidden = monitor;
  modePicker.value = option('mode');
  const format = option('resolution')+' · '+option('fps')+' FPS · H.264';
  $('.profile').textContent = $('#summary-format').textContent = format;
  const duration = +option('seconds');
  const time = [Math.floor(duration/60), duration%60].map(n=>String(n).padStart(2,'0')).join(':');
  $('#summary-time').value = time;
  const height = parseInt(option('resolution'),10);
  $('#dimensions').textContent = (height * 16 / 9)+' × '+height+' px';
  $('#estimate').textContent = '≈ '+Math.round(duration*option('rate')/8)+' MB';
}
function remember(key,value){
  try {localStorage.setItem('cut.design.'+key,value);} catch {}
}
function saved(key){
  try {return localStorage.getItem('cut.design.'+key);} catch {return null;}
}
for(const radio of profile.querySelectorAll('[type=radio]')){
  if(saved(radio.name)===radio.value) radio.checked=true;
}
profile.onchange = event=>{
  if(event.target.name) remember(event.target.name,event.target.value);
  updateProfile();
};
modePicker.onchange = ()=>{
  profile.elements.mode.value=modePicker.value;
  remember('mode',modePicker.value);
  updateProfile();
};
for(const input of [$('#game'),$('#monitor'),$('#startup'),$('#mic-mono'),$('.source')]){
  const key=input.id||'device';
  const value=saved(key);
  if(value!==null){
    if(input.type==='checkbox') input.checked=value==='true';
    else if([...input.options].some(opt=>opt.value===value)) input.value=value;
  }
  input.addEventListener('change',()=>remember(key,input.type==='checkbox'?input.checked:input.value));
}
updateProfile();
function previewSave(){
  const target=option('mode')==='Monitor completo'?$('#monitor').value:$('#game').value;
  feedback.value='Ejemplo de captura · '+option('seconds')+' s · '+target+' · '+option('resolution');
}
function drawScale(){
  const count=Math.round(Number(globalGain.value)/5);
  $('#gain-scale').textContent='['+'#'.repeat(count)+'·'.repeat(40-count)+']';
}
document.addEventListener('input',drawScale);
drawScale();
