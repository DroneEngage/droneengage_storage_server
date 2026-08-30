"use strict";

/**
 * Shared admin session store + middleware for the storage server dashboard.
 *
 * Mirrors droneengage_authenticator/src/helpers/js_admin_session.js.
 * The session middleware is used by the admin Express router.
 */

const session = require('express-session');

const DEFAULT_SESSION_SECRET = 'change-this-secret-in-production';

const sessionSecret = global.m_serverconfig.m_configuration.session_secret;
if (!sessionSecret || sessionSecret === DEFAULT_SESSION_SECRET) {
    console.log(global.Colors.BError + '[WARNING]' + global.Colors.BFgYellow +
        ' session_secret ' + global.Colors.Reset +
        ' is missing or still set to the default value in the config file. ' +
        'Set a unique, high-entropy secret before using in production.');
}

const store = new session.MemoryStore();

const sessionMiddleware = session({
    secret: sessionSecret,
    resave: false,
    saveUninitialized: false,
    store: store,
    cookie: {
        secure: global.m_serverconfig.m_configuration.enable_SSL || false,
        httpOnly: true,
        sameSite: 'strict',
        maxAge: 2 * 60 * 60 * 1000 // 2 hours
    }
});

module.exports = {
    sessionMiddleware: sessionMiddleware,
    store: store,
    sessionSecret: sessionSecret
};
