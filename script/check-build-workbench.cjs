// Fast source/asset checks, not a replacement for the Windows cargo check or GPUI tests.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const root = path.resolve(__dirname, '..');
const read = name => fs.readFileSync(path.join(root, name), 'utf8');

for (const [name, width, height] of [
  ['crates/knightcode_onboarding/assets/build-knight.png', 254, 568],
  ['crates/agent_ui/assets/build-agent-knight.png', 78, 99],
  ['crates/project_panel/assets/build-sidebar.png', 240, 354],
]) {
  const bytes = fs.readFileSync(path.join(root, name));
  assert.equal(bytes.subarray(0, 8).toString('hex'), '89504e470d0a1a0a', `${name}: PNG signature`);
  assert.equal(bytes.readUInt32BE(16), width, `${name}: artwork width`);
  assert.equal(bytes.readUInt32BE(20), height, `${name}: artwork height`);
  assert.ok(bytes.length < 500_000, `${name}: artwork should stay compact`);
}

const family = JSON.parse(read('assets/themes/knightcode-workbench.json'));
const theme = family.themes.find(theme => theme.name === 'KnightCode Workbench');
assert.equal(theme.appearance, 'dark');
for (const key of ['background', 'panel.background', 'editor.background', 'terminal.background']) {
  assert.match(theme.style[key], /^#[0-9a-f]{6}$/i, `opaque ${key}`);
}
function checkColors(value) {
  for (const child of Object.values(value)) {
    if (child && typeof child === 'object') checkColors(child);
    else if (typeof child === 'string' && child.startsWith('#')) assert.match(child, /^#[0-9a-f]{6}([0-9a-f]{2})?$/i);
  }
}
checkColors(theme.style);

const welcome = read('crates/knightcode_onboarding/src/build_welcome.rs');
const composer = read('crates/agent_ui/src/conversation_view/thread_view/build_presentation.rs');
assert.ok(welcome.includes('ToggleFileFinder') && welcome.includes('AgentPanel'), 'welcome connects to native workspace actions');
assert.ok(composer.includes('render_connected_send_button') && composer.includes('insert_context_type("file"'), 'composer uses the connected sender and real attachment editor');
assert.ok(composer.includes('What should we build?'), 'Build greeting matches the supplied reference');
for (const source of [welcome, composer]) {
  assert.ok(!source.includes('CAD_RAG'), 'project names must come from the real Explorer');
  assert.ok(!source.includes('Grok 4.20'), 'model names must come from the live model selector');
}
const tests = read('crates/agent_ui/src/conversation_view.rs');
for (const test of [
  'test_build_workbench_preserves_session_and_draft_across_chat',
  'test_build_workbench_sends_and_receives_on_the_connected_session',
]) assert.ok(tests.includes(`async fn ${test}`), `missing GPUI regression: ${test}`);

console.log('Native Build source/artwork checks passed (Rust type-check and runtime verification still required).');
