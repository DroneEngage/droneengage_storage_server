"use strict";

/**
 * Admin router for the storage server dashboard.
 *
 * Mirrors droneengage_authenticator/src/routes/js_router_admin.js:
 *   - express-session + CSRF + helmet CSP + rate limiting
 *   - login/logout with bcrypt (supports $$HASH$$('...') directive)
 *   - optional GUID-gated hidden URL
 *   - account lockout on repeated failures
 *   - read-only JSON API + EJS pages backed by DatabaseManager
 *
 * The dashboard is primarily READ-ONLY: write operations normally stay on the
 * authenticated WebSocket S2S path. A small number of super-admin maintenance
 * operations are a deliberate exception (access_log purge, News create/disable) —
 * these are session-authenticated + CSRF-protected and, for News, immediately
 * broadcast to connected comm servers so the change propagates live.
 */

const express = require('express');
const router = express.Router();
const rateLimit = require('express-rate-limit');
const csrf = require('csurf');
const helmet = require('helmet');
const bcrypt = require('bcryptjs');

const { isValidAdminUsername, isValidAdminPassword } = require('droneengage_server_common').helpers.validation;
const { sessionMiddleware } = require('./js_admin_session');
const { isBcryptHash, handleConfig } = require('droneengage_server_common').configHandler;

// Process $$HASH$$ directives in the config (admin_password) once at load.
// global.m_serverconfig.m_configuration is already parsed by server.js.
try {
    const path = require('path');
    const configFilePath = path.join(__dirname, '..', '..', global.m_serverconfig.getFileName());
    handleConfig(global.m_serverconfig.m_configuration, configFilePath);
} catch (e) {
    console.error('[dashboard] Config hash handling skipped: ' + e.message);
}

// Session
router.use(sessionMiddleware);

// CSP
router.use(helmet.contentSecurityPolicy({
    directives: {
        defaultSrc: ["'self'"],
        scriptSrc: ["'self'", "'unsafe-inline'"],
        scriptSrcAttr: ["'unsafe-inline'"],
        styleSrc: ["'self'", "'unsafe-inline'"],
        imgSrc: ["'self'", "data:"],
        fontSrc: ["'self'"],
        connectSrc: ["'self'"],
        frameSrc: ["'none'"],
        objectSrc: ["'none'"],
        baseUri: ["'self'"],
        formAction: ["'self'"],
        frameAncestors: ["'none'"]
    }
}));

// Rate limiters
const loginLimiter = rateLimit({
    windowMs: 15 * 60 * 1000,
    max: 5,
    message: { error: 'Too many login attempts, please try again later' }
});

const apiLimiter = rateLimit({
    windowMs: 1 * 60 * 1000,
    max: 60,
    standardHeaders: true,
    legacyHeaders: false,
    message: { error: 1, errorMessage: 'Too many API requests, please slow down.' }
});

// Account lockout tracking
const failedAttempts = new Map();
const LOCKOUT_DURATION = 30 * 60 * 1000;
const FAILED_ATTEMPTS_MAX_ENTRIES = 10000;

function isAccountLocked(ip, username) {
    const key = `${ip}:${username}`;
    const record = failedAttempts.get(key);
    if (!record) return false;
    const timeSinceLastAttempt = Date.now() - record.lastAttempt;
    if (timeSinceLastAttempt > LOCKOUT_DURATION) {
        failedAttempts.delete(key);
        return false;
    }
    return record.count >= 5;
}

function recordFailedAttempt(ip, username) {
    const key = `${ip}:${username}`;
    const record = failedAttempts.get(key) || { count: 0, lastAttempt: 0 };
    record.count++;
    record.lastAttempt = Date.now();
    failedAttempts.set(key, record);
    enforceFailedAttemptsCap();
}

function clearFailedAttempts(ip, username) {
    failedAttempts.delete(`${ip}:${username}`);
}

setInterval(function sweepFailedAttempts() {
    const now = Date.now();
    for (const [key, record] of failedAttempts) {
        if (now - record.lastAttempt > LOCKOUT_DURATION) {
            failedAttempts.delete(key);
        }
    }
}, 5 * 60 * 1000).unref();

function enforceFailedAttemptsCap() {
    if (failedAttempts.size <= FAILED_ATTEMPTS_MAX_ENTRIES) return;
    const entries = Array.from(failedAttempts.entries())
        .map(([key, record]) => [key, record.lastAttempt])
        .sort((a, b) => a[1] - b[1]);
    const excess = failedAttempts.size - FAILED_ATTEMPTS_MAX_ENTRIES;
    for (let i = 0; i < excess; i++) {
        failedAttempts.delete(entries[i][0]);
    }
}

// CSRF
const csrfProtection = csrf({ cookie: true });

// Helper: build an admin URL that includes the GUID prefix when configured
function adminPath(p) {
    const guid = global.m_serverconfig.m_configuration.dashboard_url_guid;
    if (guid) return '/admin/' + guid + p;
    return '/admin' + p;
}

// Auth middleware
function requireAuth(req, res, next) {
    if (req.session && req.session.adminAuthenticated) {
        return next();
    }
    return res.redirect(adminPath('/login'));
}

// GUID gate
router.use((req, res, next) => {
    const configuredGuid = global.m_serverconfig.m_configuration.dashboard_url_guid;
    if (!configuredGuid) {
        return next();
    }
    const guidPrefix = '/' + configuredGuid;
    if (req.path === guidPrefix || req.path.startsWith(guidPrefix + '/')) {
        req.url = req.url.substring(guidPrefix.length) || '/';
        if (req.url === '/') {
            return res.redirect(adminPath('/login'));
        }
        return next();
    }
    if (req.session && req.session.adminAuthenticated) {
        return next();
    }
    return res.status(404).render('pages/404', { title: '404', message: 'Not found.' });
});

// CSRF globally
router.use(csrfProtection);
router.use((req, res, next) => {
    res.locals.csrfToken = req.csrfToken();
    next();
});

// ─── Pages ────────────────────────────────────────────────────────────────────

// Login page
router.get('/login', (req, res) => {
    res.render('admin/login', {
        csrfToken: req.csrfToken(),
        error: req.session.error,
        title: 'Storage Admin Login',
        adminPath: adminPath('')
    });
    req.session.error = null;
});

// Login authentication
router.post('/login', loginLimiter, (req, res) => {
    const { username, password } = req.body;
    const config = global.m_serverconfig.m_configuration;
    const clientIp = req.ip || req.connection.remoteAddress;

    if (!isValidAdminUsername(username)) {
        req.session.error = 'Invalid username format';
        return res.redirect(adminPath('/login'));
    }
    if (!isValidAdminPassword(password)) {
        req.session.error = 'Invalid password format';
        return res.redirect(adminPath('/login'));
    }
    if (isAccountLocked(clientIp, username)) {
        req.session.error = 'Account temporarily locked due to too many failed attempts. Please try again later.';
        return res.redirect(adminPath('/login'));
    }

    if (username === config.admin_username) {
        const stored = config.admin_password;
        const match = isBcryptHash(stored)
            ? bcrypt.compareSync(password, stored)
            : (password === stored);

        if (!match) {
            recordFailedAttempt(clientIp, username);
            req.session.error = 'Invalid username or password';
            return res.redirect(adminPath('/login'));
        }

        if (!isBcryptHash(stored)) {
            console.warn(global.Colors.FgYellow +
                '[WARN] admin_password is stored in plaintext. ' +
                'Use $$HASH$$(\'...\') in server.config and restart to hash it.' +
                global.Colors.Reset);
        }

        clearFailedAttempts(clientIp, username);
        req.session.regenerate(function (err) {
            if (err) {
                console.error('Error regenerating session:', err);
                req.session.error = 'Login failed, please try again';
                return res.redirect(adminPath('/login'));
            }
            req.session.adminAuthenticated = true;
            req.session.adminUsername = username;
            req.session.save(function (saveErr) {
                if (saveErr) console.error('Error saving regenerated session:', saveErr);
                return res.redirect(adminPath('/dashboard'));
            });
        });
        return;
    }

    recordFailedAttempt(clientIp, username);
    req.session.error = 'Invalid username or password';
    return res.redirect(adminPath('/login'));
});

// Logout
router.post('/logout', (req, res) => {
    req.session.destroy((err) => {
        if (err) console.error('Error destroying session:', err);
        res.redirect(adminPath('/login'));
    });
});

// Dashboard (protected)
router.get('/dashboard', requireAuth, (req, res) => {
    res.render('admin/dashboard', {
        title: 'Storage Dashboard',
        adminUsername: req.session.adminUsername,
        dashboardGuid: global.m_serverconfig.m_configuration.dashboard_url_guid || null,
        serverId: global.m_serverconfig.m_configuration.server_id,
        dbPath: global.m_serverconfig.m_configuration.database.path
    });
});

// Messages (access_log) page
router.get('/messages', requireAuth, (req, res) => {
    res.render('admin/messages', {
        title: 'Access Log (Messages)',
        adminUsername: req.session.adminUsername,
        dashboardGuid: global.m_serverconfig.m_configuration.dashboard_url_guid || null
    });
});

// Units page
router.get('/units', requireAuth, (req, res) => {
    res.render('admin/units', {
        title: 'Units',
        adminUsername: req.session.adminUsername,
        dashboardGuid: global.m_serverconfig.m_configuration.dashboard_url_guid || null
    });
});

// Tasks page
router.get('/tasks', requireAuth, (req, res) => {
    res.render('admin/tasks', {
        title: 'Tasks',
        adminUsername: req.session.adminUsername,
        dashboardGuid: global.m_serverconfig.m_configuration.dashboard_url_guid || null
    });
});

// Missions page
router.get('/missions', requireAuth, (req, res) => {
    res.render('admin/missions', {
        title: 'Missions',
        adminUsername: req.session.adminUsername,
        dashboardGuid: global.m_serverconfig.m_configuration.dashboard_url_guid || null
    });
});

// News page
router.get('/news', requireAuth, (req, res) => {
    res.render('admin/news', {
        title: 'News',
        adminUsername: req.session.adminUsername,
        dashboardGuid: global.m_serverconfig.m_configuration.dashboard_url_guid || null
    });
});

// Queue page
router.get('/queue', requireAuth, (req, res) => {
    res.render('admin/queue', {
        title: 'Offline Queue',
        adminUsername: req.session.adminUsername,
        dashboardGuid: global.m_serverconfig.m_configuration.dashboard_url_guid || null
    });
});

// ─── API ──────────────────────────────────────────────────────────────────────

router.use('/api', apiLimiter);

// The DatabaseManager instance is attached to the router at mount time
// (see js_dashboard_server.js).  Expose it via req.app.locals.db.
function getDb(req) {
    return req.app.locals.db;
}

// API: stats (overview cards)
router.get('/api/stats', requireAuth, (req, res) => {
    try {
        const db = getDb(req);
        if (!db) return res.json({ error: 1, errorMessage: 'Database not available' });
        const stats = db.getStats();
        const queueStats = db.getQueueStats();
        const wsServer = req.app.locals.wsServer;
        res.json({
            error: 0,
            stats: {
                ...stats,
                pendingMessages: queueStats.pendingMessages,
                affectedUnits: queueStats.affectedUnits,
                connections: wsServer ? wsServer.getConnectionCount() : 0,
                dbSizeMB: (stats.dbSize / 1024 / 1024).toFixed(2)
            }
        });
    } catch (error) {
        console.error('Error fetching stats:', error);
        res.json({ error: 1, errorMessage: 'Failed to fetch stats' });
    }
});

// API: access log (messages) — paginated, filterable
router.get('/api/messages', requireAuth, (req, res) => {
    try {
        const db = getDb(req);
        if (!db) return res.json({ error: 1, errorMessage: 'Database not available' });
        const result = db.getAccessLogPage({
            page: parseInt(req.query.page) || 1,
            limit: parseInt(req.query.limit) || 50,
            action: req.query.action || null,
            unitId: req.query.unitId || null
        });
        res.json({ error: 0, ...result });
    } catch (error) {
        console.error('Error fetching messages:', error);
        res.json({ error: 1, errorMessage: 'Failed to fetch messages' });
    }
});

// API: physically delete access_log rows.  Optional action/unitId filters
// restrict the deletion to a subset; without filters the whole log is wiped.
// This is the only write operation exposed by the dashboard and is intended
// for manual maintenance (e.g. purging an oversized audit trail).
router.delete('/api/messages', requireAuth, (req, res) => {
    try {
        const db = getDb(req);
        if (!db) return res.json({ error: 1, errorMessage: 'Database not available' });
        const action = (req.query.action || '').trim() || null;
        const unitId = (req.query.unitId || '').trim() || null;
        const result = db.clearAccessLog({ action, unitId });
        console.log(`[dashboard] access_log cleared by ${req.session.adminUsername}: ` +
            `${result.deleted} rows removed` +
            (action ? ` (action=${action})` : '') +
            (unitId ? ` (unitId=${unitId})` : ''));
        res.json({ error: 0, ...result });
    } catch (error) {
        console.error('Error clearing messages:', error);
        res.json({ error: 1, errorMessage: 'Failed to clear messages' });
    }
});

// API: units — paginated, searchable
router.get('/api/units', requireAuth, (req, res) => {
    try {
        const db = getDb(req);
        if (!db) return res.json({ error: 1, errorMessage: 'Database not available' });
        const result = db.getUnitsPage({
            page: parseInt(req.query.page) || 1,
            limit: parseInt(req.query.limit) || 50,
            search: (req.query.search || '').trim() || null
        });
        res.json({ error: 0, ...result });
    } catch (error) {
        console.error('Error fetching units:', error);
        res.json({ error: 1, errorMessage: 'Failed to fetch units' });
    }
});

// API: tasks — paginated, filterable by unit
router.get('/api/tasks', requireAuth, (req, res) => {
    try {
        const db = getDb(req);
        if (!db) return res.json({ error: 1, errorMessage: 'Database not available' });
        const result = db.getTasksPage({
            page: parseInt(req.query.page) || 1,
            limit: parseInt(req.query.limit) || 50,
            unitId: req.query.unitId || null,
            includeDisabled: req.query.includeDisabled !== 'false'
        });
        res.json({ error: 0, ...result });
    } catch (error) {
        console.error('Error fetching tasks:', error);
        res.json({ error: 1, errorMessage: 'Failed to fetch tasks' });
    }
});

// API: missions — paginated, filterable by unit/account
router.get('/api/missions', requireAuth, (req, res) => {
    try {
        const db = getDb(req);
        if (!db) return res.json({ error: 1, errorMessage: 'Database not available' });
        const result = db.getMissionsPage({
            page: parseInt(req.query.page) || 1,
            limit: parseInt(req.query.limit) || 50,
            unitId: req.query.unitId || null,
            accountId: req.query.accountId || null
        });
        res.json({ error: 0, ...result });
    } catch (error) {
        console.error('Error fetching missions:', error);
        res.json({ error: 1, errorMessage: 'Failed to fetch missions' });
    }
});

// API: offline queue — paginated, filterable by unit/status
router.get('/api/queue', requireAuth, (req, res) => {
    try {
        const db = getDb(req);
        if (!db) return res.json({ error: 1, errorMessage: 'Database not available' });
        const result = db.getQueuePage({
            page: parseInt(req.query.page) || 1,
            limit: parseInt(req.query.limit) || 50,
            unitId: req.query.unitId || null,
            status: req.query.status || null
        });
        res.json({ error: 0, ...result });
    } catch (error) {
        console.error('Error fetching queue:', error);
        res.json({ error: 1, errorMessage: 'Failed to fetch queue' });
    }
});

// API: distinct unit IDs (for filter dropdowns)
router.get('/api/units/list', requireAuth, (req, res) => {
    try {
        const db = getDb(req);
        if (!db) return res.json({ error: 1, errorMessage: 'Database not available' });
        res.json({ error: 0, unitIds: db.getAllUnitIds() });
    } catch (error) {
        console.error('Error fetching unit list:', error);
        res.json({ error: 1, errorMessage: 'Failed to fetch unit list' });
    }
});

// API: distinct mission account IDs (for filter dropdowns)
router.get('/api/missions/accounts', requireAuth, (req, res) => {
    try {
        const db = getDb(req);
        if (!db) return res.json({ error: 1, errorMessage: 'Database not available' });
        res.json({ error: 0, accounts: db.getMissionAccounts() });
    } catch (error) {
        console.error('Error fetching accounts:', error);
        res.json({ error: 1, errorMessage: 'Failed to fetch accounts' });
    }
});

// API: news — paginated, filterable by scope/account
router.get('/api/news', requireAuth, (req, res) => {
    try {
        const db = getDb(req);
        if (!db) return res.json({ error: 1, errorMessage: 'Database not available' });
        const result = db.getNewsPage({
            page: parseInt(req.query.page) || 1,
            limit: parseInt(req.query.limit) || 50,
            scope: req.query.scope || null,
            accountId: req.query.accountId || null,
            includeDisabled: req.query.includeDisabled !== 'false'
        });
        res.json({ error: 0, ...result });
    } catch (error) {
        console.error('Error fetching news:', error);
        res.json({ error: 1, errorMessage: 'Failed to fetch news' });
    }
});

// API: distinct account IDs that have account-scoped news (for filter dropdowns)
router.get('/api/news/accounts', requireAuth, (req, res) => {
    try {
        const db = getDb(req);
        if (!db) return res.json({ error: 1, errorMessage: 'Database not available' });
        res.json({ error: 0, accounts: db.getNewsAccounts() });
    } catch (error) {
        console.error('Error fetching news accounts:', error);
        res.json({ error: 1, errorMessage: 'Failed to fetch news accounts' });
    }
});

// News writes (create/disable) are, like the access_log purge above, a deliberate
// exception to the "dashboard is read-only" rule: they are super-admin-only,
// session-authenticated + CSRF-protected maintenance operations. Every write is
// immediately fanned out live to connected comm servers via wsServer.broadcast()
// so GCS clients don't have to wait for their periodic resync.
const CONST_TYPE_AndruavSystem_NewsPush = 9018;

function broadcastNewsPush(req, newsPayload) {
    const wsServer = req.app.locals.wsServer;
    if (!wsServer) return;
    wsServer.broadcast({ mt: CONST_TYPE_AndruavSystem_NewsPush, ms: { news: newsPayload }, success: true, timestamp: Date.now() });
}

// API: create/update a news item (super-admin only, scope can be 'global' or 'account')
router.post('/api/news', requireAuth, (req, res) => {
    try {
        const db = getDb(req);
        if (!db) return res.json({ error: 1, errorMessage: 'Database not available' });

        const { scope, accountId, title, body, priority, expiresAt } = req.body;
        if (scope !== 'global' && scope !== 'account') {
            return res.json({ error: 1, errorMessage: "scope must be 'global' or 'account'" });
        }
        if (scope === 'account' && !accountId) {
            return res.json({ error: 1, errorMessage: 'accountId is required for scope=account' });
        }
        if (!body) {
            return res.json({ error: 1, errorMessage: 'body is required' });
        }

        const newsId = req.body.newsId || require('crypto').randomUUID();
        db.saveNews(newsId, scope, scope === 'global' ? null : accountId, title || null, body, parseInt(priority) || 0, req.session.adminUsername, expiresAt || null);
        const savedNews = db.getNews(newsId);

        console.log(`[dashboard] news ${newsId} saved by ${req.session.adminUsername} (scope=${scope})`);
        broadcastNewsPush(req, savedNews);

        res.json({ error: 0, newsId: newsId });
    } catch (error) {
        console.error('Error saving news:', error);
        res.json({ error: 1, errorMessage: 'Failed to save news' });
    }
});

// API: disable (soft-delete) a news item
router.delete('/api/news/:id', requireAuth, (req, res) => {
    try {
        const db = getDb(req);
        if (!db) return res.json({ error: 1, errorMessage: 'Database not available' });

        const newsId = req.params.id;
        const result = db.disableNews(newsId);

        console.log(`[dashboard] news ${newsId} disabled by ${req.session.adminUsername}`);
        broadcastNewsPush(req, { id: newsId, disabled: 1 });

        res.json({ error: 0, disabled: result.changes });
    } catch (error) {
        console.error('Error disabling news:', error);
        res.json({ error: 1, errorMessage: 'Failed to disable news' });
    }
});

// ─── CSRF Error Handler ──────────────────────────────────────────────────────
router.use(function (err, req, res, next) {
    if (err.code !== 'EBADCSRFTOKEN') {
        return next(err);
    }
    if (req.path.startsWith('/api/')) {
        return res.status(403).json({ error: 1, errorMessage: 'Invalid or missing CSRF token' });
    }
    req.session.error = 'Session expired, please log in again';
    return res.redirect(adminPath('/login'));
});

module.exports = router;
