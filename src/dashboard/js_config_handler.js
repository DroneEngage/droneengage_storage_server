"use strict";

/**
 * Config Handler for the storage server dashboard.
 *
 * Mirrors droneengage_authenticator/src/helpers/js_config_handler.js.
 * Processes the $$HASH$$('plaintext') directive in admin_password,
 * replacing it with a bcrypt hash in memory and persisting to the config file.
 */

const fs = require('fs');
const path = require('path');
const bcrypt = require('bcryptjs');

const BCRYPT_SALT_ROUNDS = 10;

const C = {
    Log:     () => global.Colors ? global.Colors.Log     : '',
    Success: () => global.Colors ? global.Colors.Success : '',
    Error:   () => global.Colors ? global.Colors.Error   : '',
    Warn:    () => global.Colors ? global.Colors.FgYellow : ''
};

const HASH_PATTERN_MEMORY = /\$\$HASH\$\$\(\s*['"]([^'"]+)['"]\s*\)/;
const HASH_PATTERN_FILE = /\$\$HASH\$\$\(\s*(?:'([^']+)'|\\?"([^"\\]+)\\?")\s*\)/g;

function extractHashDirective(value) {
    if (typeof value !== 'string') return null;
    const m = value.match(HASH_PATTERN_MEMORY);
    return m ? m[1] : null;
}

function isBcryptHash(value) {
    return typeof value === 'string' && /^\$2[abxy]\$\d{2}\$/.test(value);
}

function processConfigInMemory(config) {
    let changed = false;
    for (const key of Object.keys(config)) {
        const val = config[key];
        if (typeof val !== 'string') continue;

        const plaintext = extractHashDirective(val);
        if (plaintext === null) continue;

        const hash = bcrypt.hashSync(plaintext, BCRYPT_SALT_ROUNDS);
        config[key] = hash;
        changed = true;
        console.log(C.Log() +
            '[ConfigHandler] Hashed sensitive config value: ' + key +
            (global.Colors ? global.Colors.Reset : ''));
    }
    return changed;
}

function persistHashes(configFilePath) {
    let raw;
    try {
        raw = fs.readFileSync(configFilePath, 'utf8');
    } catch (err) {
        console.error(C.Error() +
            '[ConfigHandler] Cannot read config file for persistence: ' +
            err.message + (global.Colors ? global.Colors.Reset : ''));
        return false;
    }

    let changed = false;
    const updated = raw.replace(HASH_PATTERN_FILE, function (match, sq, dq) {
        const plaintext = sq !== undefined ? sq : dq;
        if (!plaintext) return match;
        changed = true;
        return bcrypt.hashSync(plaintext, BCRYPT_SALT_ROUNDS);
    });

    if (!changed) return false;

    try {
        fs.writeFileSync(configFilePath, updated, 'utf8');
        console.log(C.Success() +
            '[ConfigHandler] Config file updated — hashed values persisted.' +
            (global.Colors ? global.Colors.Reset : ''));
    } catch (err) {
        console.error(C.Error() +
            '[ConfigHandler] Failed to write config file: ' + err.message +
            (global.Colors ? global.Colors.Reset : ''));
    }
    return changed;
}

function handleConfig(config, configFilePath) {
    const changed = processConfigInMemory(config);
    if (changed) {
        persistHashes(configFilePath);
    }
}

module.exports = {
    handleConfig,
    processConfigInMemory,
    persistHashes,
    extractHashDirective,
    isBcryptHash,
    HASH_PATTERN_MEMORY
};
