const WebSocket = require('ws');
const fs = require('fs');
const path = require('path');
const https = require('https');
const http = require('http');
const logger = require('./logger');
const s2sAuth = require('./js_s2s_auth.js');
const serverConfig = require('../js_serverConfig.js');

class WebSocketServer {
  constructor(messageHandlers) {
    const config = serverConfig.m_configuration;
    this.config = config;
    this.port = process.env.de_storage_server_port || config.server_port;
    this.host = config.server_ip;
    this.enableSSL = config.enable_SSL;
    this.messageHandlers = messageHandlers;
    
    this.wss = null;
    this.connections = new Map(); // Map of connectionId -> connection info
    this.connectionCounter = 0;
  }

  /**
   * Start WebSocket server
   */
  start() {
    let wserver;

    // Create HTTP or HTTPS server based on SSL configuration
    if (this.enableSSL) {
      if (this.config.ssl_cert_file && this.config.ssl_key_file) {
        const certPath = path.resolve(__dirname, '..', this.config.ssl_cert_file);
        const keyPath = path.resolve(__dirname, '..', this.config.ssl_key_file);

        if (!fs.existsSync(certPath)) {
          logger.error(`SSL certificate file not found: ${certPath}`);
          process.exit(1);
        }

        if (!fs.existsSync(keyPath)) {
          logger.error(`SSL key file not found: ${keyPath}`);
          process.exit(1);
        }

        const options = {
          key: fs.readFileSync(keyPath),
          cert: fs.readFileSync(certPath)
        };

        wserver = https.createServer(options);
        logger.info('SSL/TLS enabled for WebSocket server');
      } else {
        logger.warn('SSL enabled but cert/key files not configured, falling back to HTTP');
        wserver = http.createServer();
      }
    } else {
      wserver = http.createServer();
    }

    wserver.listen(this.port, this.host);

    this.wss = new WebSocket.Server({ server: wserver });

    this.wss.on('listening', () => {
      logger.info(`WebSocket server listening on ${this.host}:${this.port}`);
    });

    this.wss.on('connection', (ws, req) => {
      this.handleConnection(ws, req);
    });

    this.wss.on('error', (error) => {
      logger.error(`WebSocket server error: ${error.message}`);
    });

    // No server-side ping/pong — the comm server tracks connection liveness
    // via its own onClose/reconnect logic and heartbeat to AUTH.
  }

  /**
   * Handle new WebSocket connection
   */
  handleConnection(ws, req) {
    const connectionId = ++this.connectionCounter;
    const clientIp = req.socket.remoteAddress;

    logger.info(`New connection from ${clientIp}, connectionId: ${connectionId}`);

    // Store connection info
    const connectionInfo = {
      id: connectionId,
      ws: ws,
      ip: clientIp,
      authenticated: false,
      commServerId: null,
      connectedAt: Date.now(),
      authPending: true
    };

    this.connections.set(connectionId, connectionInfo);

    // Setup connection event handlers
    ws.on('message', (data) => {
      this.handleMessage(connectionId, data);
    });

    ws.on('close', (code, reason) => {
      this.handleDisconnection(connectionId, code, reason);
    });

    ws.on('error', (error) => {
      logger.error(`Connection ${connectionId} error: ${error.message}`);
    });

    // Send S2S auth challenge only if s2s_cert_enabled
    if (this.config.s2s_cert_enabled === true) {
      this.sendAuthChallenge(connectionId);
    } else {
      // If S2S cert auth is disabled, mark as authenticated immediately
      connectionInfo.authenticated = true;
      connectionInfo.authPending = false;
      logger.info(`Connection ${connectionId} accepted without S2S auth (s2s_cert_enabled=false)`);
    }
  }

  /**
   * Send S2S auth challenge to connecting comm server
   */
  sendAuthChallenge(connectionId) {
    const connection = this.connections.get(connectionId);
    if (!connection) return;

    const nonce = s2sAuth.fn_generateNonce();
    connection.authNonce = nonce;
    connection.authChallengeSent = Date.now();

    const challenge = s2sAuth.fn_buildChallenge(nonce);
    connection.ws.send(challenge);

    logger.debug(`Sent S2S auth challenge to connection ${connectionId}`);

    // Set timeout for auth response
    setTimeout(() => {
      if (connection.authPending) {
        logger.warn(`S2S auth timeout for connection ${connectionId}`);
        this.close(connectionId, 1008, 'Auth timeout');
      }
    }, s2sAuth.CONST_S2S_AUTH_HANDSHAKE_TIMEOUT);
  }

  /**
   * Handle incoming message
   */
  async handleMessage(connectionId, data) {
    const connection = this.connections.get(connectionId);
    if (!connection) {
      logger.warn(`Message from unknown connection ${connectionId}`);
      return;
    }

    try {
      // Check for S2S auth envelope first
      const envelope = s2sAuth.fn_parseEnvelope(data);
      
      if (envelope) {
        // Handle S2S auth message
        if (envelope.s2s_auth === s2sAuth.CONST_S2S_AUTH_RESPONSE) {
          await this.handleS2SAuthResponse(connectionId, envelope);
        }
        return;
      }

      // Handle normal message
      const message = JSON.parse(data);
      logger.debug(`Message from connection ${connectionId}, type: ${message.mt}`);

      // Check if authenticated
      if (!connection.authenticated) {
        this.sendError(connectionId, 'Not authenticated');
        this.close(connectionId, 1008, 'Authentication required');
        return;
      }

      // Route to appropriate handler by numeric type (mt field)
      if (this.messageHandlers[message.mt]) {
        await this.messageHandlers[message.mt](connectionId, message);
      } else {
        logger.warn(`Unknown message type: ${message.mt}`);
        this.sendError(connectionId, `Unknown message type: ${message.mt}`);
      }
    } catch (error) {
      logger.error(`Error handling message from ${connectionId}: ${error.message}`);
      this.sendError(connectionId, 'Invalid message format');
    }
  }

  /**
   * Handle S2S auth response from comm server
   */
  async handleS2SAuthResponse(connectionId, envelope) {
    const connection = this.connections.get(connectionId);
    if (!connection) return;

    try {
      // Verify signature using the comm server's public key
      const isValid = s2sAuth.fn_verify(connection.authNonce, envelope.sig, envelope.id);

      if (isValid) {
        connection.authenticated = true;
        connection.commServerId = envelope.id;
        connection.authPending = false;
        
        logger.info(`Connection ${connectionId} authenticated for comm server ${envelope.id}`);
        
        this.send(connectionId, {
          type: 'auth_success',
          connectionId: connectionId,
          timestamp: Date.now()
        });
      } else {
        logger.warn(`S2S auth failed for connection ${connectionId}`);
        this.sendError(connectionId, 'Invalid signature');
        this.close(connectionId, 1008, 'Authentication failed');
      }
    } catch (error) {
      logger.error(`S2S auth error for connection ${connectionId}: ${error.message}`);
      this.sendError(connectionId, 'Authentication error');
      this.close(connectionId, 1011, 'Internal error');
    }
  }

  /**
   * Handle disconnection
   */
  handleDisconnection(connectionId, code, reason) {
    const connection = this.connections.get(connectionId);
    if (connection) {
      const duration = Date.now() - connection.connectedAt;
      logger.info(`Connection ${connectionId} disconnected (code: ${code}, reason: ${reason}, duration: ${duration}ms)`);
      this.connections.delete(connectionId);
    }
  }

  /**
   * Send message to specific connection
   */
  send(connectionId, data) {
    const connection = this.connections.get(connectionId);
    if (connection && connection.ws.readyState === WebSocket.OPEN) {
      try {
        connection.ws.send(JSON.stringify(data));
      } catch (error) {
        logger.error(`Error sending to connection ${connectionId}: ${error.message}`);
      }
    }
  }

  /**
   * Send error message
   */
  sendError(connectionId, error) {
    this.send(connectionId, {
      type: 'error',
      error: error,
      timestamp: Date.now()
    });
  }

  /**
   * Close connection
   */
  close(connectionId, code, reason) {
    const connection = this.connections.get(connectionId);
    if (connection) {
      connection.ws.close(code, reason);
    }
  }

  /**
   * Get connection info
   */
  getConnection(connectionId) {
    return this.connections.get(connectionId);
  }

  /**
   * Get all connections
   */
  getAllConnections() {
    return Array.from(this.connections.values());
  }

  /**
   * Get connection count
   */
  getConnectionCount() {
    return this.connections.size;
  }

  /**
   * Stop WebSocket server
   */
  stop() {
    if (this.wss) {
      // Close all connections
      this.connections.forEach((connection, connectionId) => {
        this.close(connectionId, 1001, 'Server shutting down');
      });

      this.wss.close(() => {
        logger.info('WebSocket server stopped');
      });
    }
  }
}

module.exports = WebSocketServer;
