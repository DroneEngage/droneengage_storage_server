const WebSocket = require('ws');
const fs = require('fs');
const path = require('path');
const logger = require('./logger');
const serverConfig = require('../js_serverConfig.js');
const s2sAuth = require('./js_s2s_auth.js');

class AuthClient {
  constructor() {
    const config = serverConfig.m_configuration;
    this.config = config;
    this.serverId = config.server_id;
    this.s2sTargetIp = config.s2s_ws_target_ip;
    this.s2sTargetPort = config.s2s_ws_target_port;
    this.enableSSL = config.enable_SSL;
    this.ws = null;
    this.connected = false;
    this.authenticated = false;
  }

  /**
   * Connect to AUTH server via S2S WebSocket
   */
  async connect() {
    try {
      const protocol = this.enableSSL ? 'wss' : 'ws';
      const url = `${protocol}://${this.s2sTargetIp}:${this.s2sTargetPort}`;
      
      logger.info(`Connecting to AUTH server at ${url}`);

      const wsOptions = {
        rejectUnauthorized: !this.config.allow_fake_SSL
      };

      if (this.enableSSL && this.config.ssl_cert_file && this.config.ssl_key_file) {
        wsOptions.cert = fs.readFileSync(path.resolve(__dirname, '..', this.config.ssl_cert_file));
        wsOptions.key = fs.readFileSync(path.resolve(__dirname, '..', this.config.ssl_key_file));
      }

      if (this.enableSSL && this.config.ca_cert_path) {
        wsOptions.ca = fs.readFileSync(path.resolve(__dirname, '..', this.config.ca_cert_path));
      }

      this.ws = new WebSocket(url, wsOptions);

      this.ws.on('open', () => {
        logger.info('Connected to AUTH server');
        this.connected = true;
        this.handleS2SAuth();
      });

      this.ws.on('message', (data) => {
        this.handleMessage(data);
      });

      this.ws.on('close', (code, reason) => {
        logger.warn(`AUTH connection closed (code: ${code}, reason: ${reason})`);
        this.connected = false;
        this.authenticated = false;
      });

      this.ws.on('error', (error) => {
        logger.error(`AUTH connection error: ${error.message}`);
      });

      return new Promise((resolve, reject) => {
        this.ws.once('open', () => resolve(true));
        this.ws.once('error', (error) => reject(error));
      });

    } catch (error) {
      logger.error(`Failed to connect to AUTH: ${error.message}`);
      throw error;
    }
  }

  /**
   * Handle S2S authentication with AUTH server
   */
  handleS2SAuth() {
    // Wait for challenge from AUTH server
    // AUTH server will send challenge, we respond with signed nonce
    logger.info('Waiting for S2S auth challenge from AUTH');
  }

  /**
   * Handle incoming message from AUTH
   */
  handleMessage(data) {
    try {
      const envelope = s2sAuth.fn_parseEnvelope(data);
      
      if (!envelope) {
        // Not an S2S auth message, handle as normal message
        this.handleNormalMessage(data);
        return;
      }

      if (envelope.s2s_auth === s2sAuth.CONST_S2S_AUTH_CHALLENGE) {
        // AUTH server sent challenge, sign and respond
        const response = s2sAuth.fn_buildResponse(envelope.nonce, this.serverId);
        this.ws.send(response);
        logger.info('Sent S2S auth response to AUTH');
      } else if (envelope.s2s_auth === s2sAuth.CONST_S2S_AUTH_RESPONSE) {
        // This shouldn't happen since we're the connecting side
        logger.warn('Received unexpected S2S auth response from AUTH');
      }

    } catch (error) {
      logger.error(`Error handling message from AUTH: ${error.message}`);
    }
  }

  /**
   * Handle normal (non-auth) message from AUTH
   */
  handleNormalMessage(data) {
    try {
      const message = JSON.parse(data);
      logger.debug(`Message from AUTH: ${message.type}`);
      
      // Handle registration confirmation, etc.
      if (message.type === 'registration_success') {
        this.authenticated = true;
        logger.info('Successfully authenticated with AUTH');
      }
    } catch (error) {
      logger.error(`Error parsing normal message: ${error.message}`);
    }
  }

  /**
   * Send message to AUTH
   */
  send(message) {
    if (this.ws && this.ws.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify(message));
    } else {
      logger.warn('Cannot send message: not connected to AUTH');
    }
  }

  /**
   * Disconnect from AUTH
   */
  disconnect() {
    if (this.ws) {
      this.ws.close();
      this.ws = null;
      this.connected = false;
      this.authenticated = false;
      logger.info('Disconnected from AUTH');
    }
  }

  /**
   * Get connection status
   */
  isConnected() {
    return this.connected;
  }

  /**
   * Get authentication status
   */
  isAuthenticated() {
    return this.authenticated;
  }

  /**
   * Get server ID
   */
  getServerId() {
    return this.serverId;
  }
}

module.exports = AuthClient;
