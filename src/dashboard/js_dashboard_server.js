"use strict";

/**
 * Dashboard HTTP server for the storage server.
 *
 * Mirrors droneengage_authenticator/src/server.js fn_startViewsServer():
 * a separate Express app on its own dashboard_port, serving the read-only
 * admin UI.  SSL reuses the storage server's existing ssl_key_file /
 * ssl_cert_file.  The DatabaseManager and WebSocketServer instances from
 * the running storage server are attached via app.locals so the router's
 * API endpoints can query them.
 */

const path = require('path');
const fs = require('fs');
const http = require('http');
const https = require('https');
const express = require('express');
const cookieParser = require('cookie-parser');
const bodyParser = require('body-parser');
const cors = require('cors');
const helmet = require('helmet');
const logger = require('../logger');

let dashboardApp = null;
let dashboardServer = null;

/**
 * Start the dashboard server.
 * @param {object} database - initialized DatabaseManager instance
 * @param {object} wsServer - initialized WebSocketServer instance
 */
function start(database, wsServer) {
    const config = global.m_serverconfig.m_configuration;

    if (!config.dashboard_enable) {
        logger.info('Dashboard disabled (dashboard_enable=false)');
        return;
    }

    const dashboardIP = config.dashboard_listening_ip;
    if (!dashboardIP) {
        console.log(global.Colors.BError + 'FATAL ERROR:' + global.Colors.FgYellow +
            ' dashboard_listening_ip ' + global.Colors.Reset +
            ' is not specified in config file.');
        process.exit(0);
    }

    const app = express();

    app.use(helmet({
        contentSecurityPolicy: {
            directives: {
                defaultSrc: ["'self'"],
                styleSrc: ["'self'", "'unsafe-inline'"],
                scriptSrc: ["'self'", "'unsafe-inline'"],
                scriptSrcAttr: ["'unsafe-inline'"],
                imgSrc: ["'self'", "data:"],
                fontSrc: ["'self'"],
                connectSrc: ["'self'"],
                frameSrc: ["'none'"],
                objectSrc: ["'none'"],
                baseUri: ["'self'"],
                formAction: ["'self'"],
                frameAncestors: ["'none'"]
            }
        }
    }));

    const dashboardPort = process.env.de_storage_dashboard_port || config.dashboard_port || 9001;
    app.set('port', dashboardPort);
    app.set('views', path.join(__dirname, '..', '..', 'views'));
    app.set('view engine', 'ejs');

    app.use(cors());
    app.use(bodyParser.json());
    app.use(bodyParser.urlencoded({ extended: false }));
    app.use(cookieParser());

    // Static assets (bootstrap)
    app.use('/public', express.static(path.join(__dirname, '..', '..', 'public')));

    // Attach the live database + ws server instances for the router to use
    app.locals.db = database;
    app.locals.wsServer = wsServer;

    // Mount the admin router under /admin
    const adminRouter = require('./js_router_admin');
    app.use('/admin', adminRouter);

    // Root → admin login
    app.get('/', (req, res) => {
        const guid = config.dashboard_url_guid;
        res.redirect(guid ? '/admin/' + guid + '/login' : '/admin/login');
    });

    // 404 for everything else
    app.use((req, res) => {
        res.status(404).render('pages/404', { title: '404', message: 'Not found.' });
    });

    // Create HTTP or HTTPS server
    if (config.enable_SSL && config.ssl_cert_file && config.ssl_key_file) {
        const certPath = path.resolve(__dirname, '..', '..', config.ssl_cert_file);
        const keyPath = path.resolve(__dirname, '..', '..', config.ssl_key_file);
        if (fs.existsSync(certPath) && fs.existsSync(keyPath)) {
            dashboardServer = https.createServer({
                key: fs.readFileSync(keyPath),
                cert: fs.readFileSync(certPath)
            }, app);
            logger.info('SSL/TLS enabled for dashboard server');
        } else {
            logger.warn('SSL enabled but cert/key not found, dashboard falling back to HTTP');
            dashboardServer = http.createServer(app);
        }
    } else {
        dashboardServer = http.createServer(app);
    }

    dashboardServer.listen(dashboardPort, dashboardIP, () => {
        console.log(global.Colors.Success + '[OK] Dashboard Server Started' + global.Colors.Reset);
        const protocol = config.enable_SSL ? 'https' : 'http';
        const guid = config.dashboard_url_guid || '';
        const loginPath = guid ? '/admin/' + guid + '/login' : '/admin/login';
        console.log(global.Colors.Log + 'Dashboard: ' + global.Colors.BSuccess +
            protocol + '://' + dashboardIP + ':' + dashboardPort + loginPath +
            global.Colors.Reset + (guid ? ' (GUID mode)' : ''));
    });

    dashboardApp = app;
}

function stop() {
    if (dashboardServer) {
        dashboardServer.close(() => {
            logger.info('Dashboard server stopped');
        });
        dashboardServer = null;
        dashboardApp = null;
    }
}

module.exports = { start, stop };
