// Read the currently committed draft payload of an isolated native-test profile.
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const stateMagic = Buffer.from('ROCKET-STATE\0');
const draftMagic = Buffer.from('ROCKET-DRAFTS\0\x01');

function readDraftPayload(directory) {
  const stateFile = path.join(directory, 'state.bin');
  for (let attempt = 0; attempt < 3; attempt++) {
    const state = fs.readFileSync(stateFile);
    assert(state.length > stateMagic.length && state.subarray(0, stateMagic.length).equals(stateMagic), 'Invalid state header');
    const version = state[stateMagic.length];
    assert(version >= 1 && version <= 18, 'Unsupported state version');
    if (version < 18) return state;
    const offset = stateMagic.length + 1;
    assert(state.length >= offset + 4, 'Truncated draft reference');
    const length = state.readUInt32LE(offset);
    assert(length <= 96 && state.length >= offset + 4 + length, 'Invalid draft reference length');
    const name = state.subarray(offset + 4, offset + 4 + length).toString('utf8');
    assert(/^drafts-[0-9]+-[0-9]+\.bin$/.test(name), 'Invalid draft archive name');
    let payload;
    try {
      payload = fs.readFileSync(path.join(directory, name));
    } catch (error) {
      // A completed autosave can retire the generation read just above. Retry
      // only when its pointer actually changed; never hide a damaged profile.
      if (error.code === 'ENOENT' && !fs.readFileSync(stateFile).equals(state)) continue;
      throw error;
    }
    assert(payload.length >= draftMagic.length + 4 && payload.subarray(0, draftMagic.length).equals(draftMagic), 'Invalid draft archive header');
    return payload;
  }
  throw new Error('State kept changing while reading the draft fixture');
}
module.exports = { readDraftPayload };
