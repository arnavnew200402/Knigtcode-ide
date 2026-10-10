// Asset/source validation is not a replacement for Rust type-checking or a
// visual comparison of the installed Windows application.
const assert = require('node:assert/strict');
const fs = require('node:fs');
for (const [path, width, height] of [
  ['crates/knightcode_onboarding/assets/home-landscape.png', 1774, 887],
]) {
  const png = fs.readFileSync(path);
  assert.equal(png.subarray(0, 8).toString('hex'), '89504e470d0a1a0a', path);
  assert.equal(png.readUInt32BE(16), width, path);
  assert.equal(png.readUInt32BE(20), height, path);
  assert(png.length < 4 * 1024 * 1024, `${path}: excessive asset size`);
}
const artwork = fs.readFileSync('crates/knightcode_onboarding/assets/home-landscape.png');
assert.equal(require('node:crypto').createHash('sha256').update(artwork).digest('hex'),
  'b497fd448e0dc0b507182e1e83db6f3d141a29a0fcdf2a8d160566cfcac869c0',
  'Use the supplied clean Home background without compositing or resampling');
const page = fs.readFileSync('crates/knightcode_onboarding/src/workspace_modes.rs', 'utf8');
const home = fs.readFileSync('crates/knightcode_onboarding/src/workspace_modes/home_presentation.rs', 'utf8');
const title = fs.readFileSync('crates/title_bar/src/title_bar.rs', 'utf8');
const platform = fs.readFileSync('crates/platform_title_bar/src/platform_title_bar.rs', 'utf8');
assert(page.includes('Page::Home => home_presentation::render(self, window, cx)'));
assert(page.includes('.when(self.page != Page::Home, |page|'));
assert(page.includes('workspace.set_knightcode_front_page(true, cx)'));
assert(page.includes('cx.defer_in(window, |item, _, cx| item.refresh_recent(cx))'));
for (const text of ['Turn your ideas', 'impact.', 'Recent projects', 'Start chatting', 'Open project', 'Last opened']) assert(home.includes(text), text);
const compactHome = home.replace(/\s+/g, '');
for (const live of ['recent.identity_paths', 'recent.paths', 'recent.timestamp', 'this.open_recent', 'ShowChat.boxed_clone()', 'workspace::Open::default()', 'OpenRecent::default()', 'write_to_clipboard']) assert(compactHome.includes(live), live);
for (const text of ['CAD-RAG', 'Typera', '11 Sep 2025', 'Arnav\\Desktop']) assert(!home.includes(text), `Baked reference data: ${text}`);
assert(home.includes('OnceLock<HomeArtwork>'), 'Artwork identity must be cached');
assert(!home.includes('overflow_y_scroll') && !home.includes('overflow_x_scroll'), 'Home must not scroll');
assert(!home.includes('flex_wrap'), 'Home card rows must not wrap into clipped content');
assert(home.includes('HomeLayout::new'), 'Use logical viewport-fit sizing');
assert(home.includes('take(layout.recent_count)'), 'Limit recent cards to available width');
assert(home.includes('.when(has_recents,'), 'Hide the entire recent-project section until actual history exists');
assert(home.includes('is_some_and(|recent| !recent.is_empty())'), 'Never fabricate recent projects');
assert(!home.includes('Loading recent projects') && !home.includes('Your recent projects will appear'), 'No empty/loading recent-project row');
assert.equal((home.match(/\bimg\(/g) || []).length, 1, 'Use one background image, not repeated card textures');
assert(home.includes('ObjectFit::Cover'), 'Do not stretch background proportions');
assert(platform.includes('KNIGHTCODE_HOME_TITLE_BAR_HEIGHT: f32 = 48.'), 'Home header must stay slim');
assert.match(page, /\.h\(px\(40\.\)\)\s*\.flex_none\(\)/, 'Chat history rows must not shrink and clip');
const conversation = fs.readFileSync('crates/agent_ui/src/conversation_view.rs', 'utf8');
assert(conversation.includes('.filter(|paths| !paths.is_empty())'), 'Normalize empty saved cwd');
assert(conversation.includes('!sent_queued_message && !self.full_page_chat'), 'No reply-completion popup in full-page Chat');
assert(conversation.includes('test_full_page_chat_restores_empty_saved_cwd'));
assert(conversation.includes('test_full_page_chat_suppresses_reply_notifications'));
assert(title.includes('render_home_title_bar'));
assert(title.includes('home-settings'));
assert(title.includes('home-global-search'));
assert(platform.includes('KNIGHTCODE_HOME_TITLE_BAR_HEIGHT'));
assert(!page.includes('refresh-recents'));
assert(!page.includes('clone-project'));
const recents = fs.readFileSync('crates/recent_projects/src/recent_projects.rs', 'utf8');
assert(recents.includes('context.contains("KnightCodeHome")'), 'Palette must be scoped to the originating Home page');
assert(recents.includes('home_palette: false'), 'Ordinary pickers must retain their existing presentation');
for (const color of ['0x061326', '0x133457', '0xf0f6ff']) assert(recents.includes(color), 'Home picker surface/selection/text');
assert(recents.includes('editor::EditorElement::new'), 'Keep the actual searchable editor');
assert(!recents.includes('GlobalTheme::update_theme'), 'Do not change the user theme to recolor a modal');
const modeSwitch = fs.readFileSync('crates/title_bar/src/chat_mode_switch.rs', 'utf8');
assert(modeSwitch.includes('.occlude()') && modeSwitch.includes('cx.stop_propagation()'), 'Mode switch must not initiate caption dragging');
assert(!modeSwitch.includes('.disabled('), 'Agent/account loading must not disable navigation');
assert(title.includes('chat_mode_switch::render(mode)'), 'Use the tested mode switch in the real title bar');
assert(modeSwitch.includes('test_knightcode_mode_switch_clicks_do_not_arm_caption_drag'));
console.log('Native Home artwork, dynamic project data, action wiring, and chrome source checks passed.');
