// xenify.net's only script.
// - Footage loops on its own, unless the visitor asked for less motion: then
//   it waits, with the usual play controls.
// - The download page lights up the visitor's system.
const still = matchMedia('(prefers-reduced-motion: reduce)');
function motion() {
  for (const v of document.querySelectorAll('video[data-loop]')) {
    if (still.matches) {
      v.pause();
      v.controls = true;
    } else {
      v.controls = false;
      v.play().catch(() => { v.controls = true; });
    }
  }
}
motion();
still.addEventListener('change', motion);

const ua = navigator.userAgent;
const system = /Windows/.test(ua) ? 'windows' : /Mac OS X|Macintosh/.test(ua) && !/iPhone|iPad/.test(ua) ? 'macos' : /Linux|X11/.test(ua) && !/Android/.test(ua) ? 'linux' : '';
const card = system && document.querySelector(`.system[data-system="${system}"]`);
if (card) {
  card.classList.add('mine');
  card.parentElement.prepend(card);
}

// Films: a play button and a step for each part (data-at, in seconds).
// Clicking a step plays from there; the step playing fills as it goes.
for (const film of document.querySelectorAll('.film')) {
  const video = film.querySelector('video');
  const play = film.querySelector('.film-play');
  const steps = [...film.querySelectorAll('.film-steps button')];
  if (!video) continue;
  const at = steps.map((b) => Number(b.dataset.at));
  const label = play?.querySelector('span');
  const start = (t) => {
    if (t != null) video.currentTime = t;
    film.classList.add('playing');
    film.classList.remove('paused');
    video.play().catch(() => {});
  };
  play?.addEventListener('click', () => {
    if (!video.paused) { video.pause(); return; }
    start(video.ended || film.classList.contains('ended') ? 0 : null);
  });
  video.addEventListener('click', () => { if (!video.paused) video.pause(); });
  video.addEventListener('pause', () => {
    if (video.ended) return;
    film.classList.add('paused');
    if (label) label.textContent = 'Carry on';
  });
  video.addEventListener('ended', () => {
    film.classList.remove('playing', 'paused');
    film.classList.add('ended');
    if (label) label.textContent = 'Play again';
    steps.forEach((b) => { b.classList.add('done'); b.style.setProperty('--done', 0); b.removeAttribute('aria-current'); });
  });
  video.addEventListener('play', () => { film.classList.remove('ended'); if (label) label.textContent = 'Pause'; });
  steps.forEach((b, i) => b.addEventListener('click', () => start(at[i])));
  video.addEventListener('timeupdate', () => {
    const t = video.currentTime;
    steps.forEach((b, i) => {
      const end = i + 1 < at.length ? at[i + 1] : video.duration || at[i] + 3;
      const now = t >= at[i] && t < end;
      b.classList.toggle('done', t >= end);
      if (now) b.setAttribute('aria-current', 'step'); else b.removeAttribute('aria-current');
      b.style.setProperty('--done', now ? Math.min(1, (t - at[i]) / (end - at[i])) : 0);
    });
  });
}

// Home: the apps, one at a time. Each film plays once (a picture shows for
// a while), then the next app's name lights up. Choosing a name stops the
// round and loops that app's film instead. With less motion asked for,
// nothing plays by itself: the names still switch, and films get controls.
const tour = document.querySelector('.tour');
if (tour) {
  const tabs = [...tour.querySelectorAll('[role=tab]')];
  const panels = tabs.map((t) => document.getElementById(t.getAttribute('aria-controls')));
  const STILL_MS = 7000;
  let now = 0, auto = true, shown = 0, seen = true;
  const media = (i) => panels[i].querySelector('video');
  const show = (i, focus) => {
    media(now)?.pause();
    tabs.forEach((t, j) => {
      const on = j === i;
      t.setAttribute('aria-selected', on);
      t.tabIndex = on ? 0 : -1;
      t.style.setProperty('--done', 0);
      t.classList.toggle('timed', on && auto && !still.matches);
      panels[j].hidden = !on;
    });
    now = i;
    shown = performance.now();
    if (focus) tabs[i].focus();
    const v = media(i);
    if (v) {
      v.loop = !auto;
      v.controls = still.matches;
      if (!still.matches && seen) { v.currentTime = 0; v.play().catch(() => { v.controls = true; }); }
    }
  };
  const next = () => show((now + 1) % tabs.length);
  for (const v of tour.querySelectorAll('video')) v.addEventListener('ended', () => { if (auto && !still.matches) next(); });
  const tick = () => {
    requestAnimationFrame(tick);
    if (!auto || still.matches || !seen) return;
    const v = media(now);
    const done = v ? (v.duration ? v.currentTime / v.duration : 0) : (performance.now() - shown) / STILL_MS;
    tabs[now].style.setProperty('--done', Math.min(1, done));
    if (!v && done >= 1) next();
  };
  const choose = (i, focus) => {
    auto = false;
    show(i, focus);
  };
  tabs.forEach((t, i) => {
    t.addEventListener('click', () => choose(i));
    t.addEventListener('keydown', (e) => {
      const to = { ArrowRight: now + 1, ArrowLeft: now - 1, Home: 0, End: tabs.length - 1 }[e.key];
      if (to == null) return;
      e.preventDefault();
      choose((to + tabs.length) % tabs.length, true);
    });
  });
  // Off screen, the round waits where it is.
  new IntersectionObserver(([e]) => {
    seen = e.isIntersecting;
    const v = media(now);
    if (!seen) { v?.pause(); return; }
    shown = performance.now() - (Number(tabs[now].style.getPropertyValue('--done')) || 0) * STILL_MS;
    if (v && !still.matches) v.play().catch(() => {});
  }).observe(tour);
  still.addEventListener('change', () => show(now));
  show(0);
  tick();
}
