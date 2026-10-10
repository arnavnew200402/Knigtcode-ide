// Exercise the source gate with both Git checkout newline formats, without
// rewriting repository files. Negative cases must still fail the real checks.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const checker = require.resolve('./check-home-design.cjs');
const read = fs.readFileSync;
function run(newline, damage = null) {
  fs.readFileSync = function (file, ...args) {
    let result = read.call(this, file, ...args);
    const path = String(file).replaceAll('\\', '/');
    if (typeof result === 'string' && path.endsWith('.rs')) {
      result = result.replace(/\r?\n/g, newline);
      if (damage === 'history' && path.endsWith('/workspace_modes.rs')) {
        result = result.replace(/\.h\(px\(40\.\)\)\s*\.flex_none\(\)/g,
          '.h(px(40.)).flex_shrink_1()');
      }
    }
    if (damage === 'artwork' && path.endsWith('/assets/home-landscape.png')) {
      result = Buffer.from(result);
      result[result.length - 20] ^= 1;
    }
    return result;
  };
  try {
    delete require.cache[checker];
    require(checker);
  } finally {
    fs.readFileSync = read;
    delete require.cache[checker];
  }
}
for (const newline of ['\n', '\r\n']) {
  run(newline);
  assert.throws(() => run(newline, 'history'), /Chat history rows must not shrink and clip/);
  assert.throws(() => run(newline, 'artwork'), /Use the supplied clean Home background/);
}
console.log('Home gate regression tests passed: LF/CRLF accepted; broken row sizing/altered artwork rejected.');
