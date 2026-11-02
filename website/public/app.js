const root = document.documentElement;
const themeButton = document.querySelector('.theme-toggle');
function setTheme(theme) {
  root.dataset.theme = theme;
  themeButton.setAttribute('aria-label', theme === 'dark' ? 'Byt till ljust tema' : 'Byt till mörkt tema');
  themeButton.querySelector('img').src = `/assets/icons/${theme === 'dark' ? 'sun' : 'moon'}.svg`;
  document.querySelector('meta[name="theme-color"]').content = theme === 'dark' ? '#181818' : '#f2f2ee';
  try { localStorage.setItem('connect-site-theme', theme); } catch { /* The site works without storage. */ }
}
try { const saved = localStorage.getItem('connect-site-theme'); if (['dark', 'light'].includes(saved)) setTheme(saved); } catch { /* Private browsing may disable storage. */ }
themeButton.addEventListener('click', () => setTheme(root.dataset.theme === 'dark' ? 'light' : 'dark'));
const preview = document.querySelector('#preview');
document.querySelectorAll('[data-view]').forEach(button => button.addEventListener('click', () => {
  preview.dataset.preview = button.dataset.view;
  document.querySelectorAll('[data-view]').forEach(item => item.setAttribute('aria-pressed', String(item === button)));
}));
document.querySelectorAll('[data-quality]').forEach(button => button.addEventListener('click', () => {
  document.querySelectorAll('[data-quality]').forEach(item => item.setAttribute('aria-pressed', String(item === button)));
  document.querySelector('.quality').innerHTML = `${button.dataset.quality} <span>·</span> OBS`;
  document.querySelector('[data-view="share"]').click();
}));
document.querySelectorAll('button[data-skin]').forEach(button => button.addEventListener('click', () => {
  document.querySelector('.skin-demo').dataset.skin = button.dataset.skin;
  document.querySelectorAll('button[data-skin]').forEach(item => item.setAttribute('aria-pressed', String(item === button)));
}));
const dialog = document.querySelector('#download-dialog');
let opener;
document.querySelectorAll('[data-download]').forEach(button => button.addEventListener('click', () => {
  opener = button;
  document.querySelector('.download-status').textContent = '';
  dialog.showModal();
  document.body.classList.add('dialog-open');
}));
dialog.querySelector('.dialog-close').addEventListener('click', () => dialog.close());
dialog.addEventListener('keydown', event => {
  if (event.key !== 'Tab') return;
  const controls = [...dialog.querySelectorAll('button:not([disabled]), a[href]')];
  const first = controls[0], last = controls.at(-1);
  if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus(); }
  else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
});
dialog.addEventListener('click', event => {
  if (event.target !== dialog) return;
  const box = dialog.getBoundingClientRect();
  if (event.clientX < box.left || event.clientX > box.right || event.clientY < box.top || event.clientY > box.bottom) dialog.close();
});
dialog.addEventListener('close', () => { document.body.classList.remove('dialog-open'); opener?.focus(); });
document.querySelector('#actual-download').addEventListener('click', () => {
  document.querySelector('.download-status').textContent = 'Nedladdningen har begärts. Du hittar filen i webbläsarens nedladdningar när den är klar.';
});
