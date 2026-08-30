"use strict";

/**
 * Validation helpers for the storage server dashboard.
 *
 * Mirrors droneengage_authenticator/src/helpers/hlp_validation.js
 * (only the admin username/password validators needed here).
 */

function isAlphanumeric(str) {
    if (str == null) return false;
    return (str.match(/^[_a-z0-9]+$/i) != null);
}

function isEmail(str) {
    if (str == null) return false;
    return (str.match(/^\w+([\.-]?\w+)*@\w+([\.-]?\w+)*(\.\w{2,4})+$/i) != null);
}

function isValidAdminUsername(username) {
    if (username == null) return false;
    if (username.length < 3 || username.length > 50) return false;
    return isAlphanumeric(username) || isEmail(username);
}

function isValidAdminPassword(password) {
    if (password == null) return false;
    if (password.length < 8 || password.length > 128) return false;
    return !/\s/.test(password);
}

module.exports = {
    isAlphanumeric,
    isEmail,
    isValidAdminUsername,
    isValidAdminPassword
};
