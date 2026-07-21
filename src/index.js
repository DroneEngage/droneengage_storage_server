const serverConfig = require('../js_serverConfig.js');
const logger = require('./logger');
const WebSocketServer = require('./websocketServer');
const DatabaseManager = require('./database');
const MessageHandlers = require('./messageHandlers');
const OfflineQueue = require('./offlineQueue');

class StorageServer {
  constructor() {
    this.config = serverConfig.m_configuration;
    this.wsServer = null;
    this.database = null;
    this.messageHandlers = null;
    this.offlineQueue = null;
    this.running = false;
  }

  /**
   * Initialize and start the storage server
   */
  async start() {
    try {
      logger.info('Starting DroneEngage Storage Server...');

      // Initialize database
      logger.info('Initializing database...');
      this.database = new DatabaseManager();
      if (!this.database.initialize()) {
        throw new Error('Database initialization failed');
      }

      // Initialize message handlers
      this.messageHandlers = new MessageHandlers(this.database, null); // wsServer will be set later

      // Initialize offline queue
      this.offlineQueue = new OfflineQueue(this.database, null);
      this.offlineQueue.startCleanupJob();

      // Initialize WebSocket server
      logger.info('Initializing WebSocket server...');
      this.wsServer = new WebSocketServer(this.messageHandlers.getHandlers());
      
      // Update message handlers with wsServer reference
      this.messageHandlers.wsServer = this.wsServer;
      this.offlineQueue.wsServer = this.wsServer;

      // Start WebSocket server
      this.wsServer.start();

      // Setup graceful shutdown
      this.setupShutdownHandlers();

      this.running = true;
      logger.info('Storage server started successfully');
      logger.info(`Server endpoint: ${this.config.public_host}:${this.config.server_port}`);
      logger.info(`Database: ${this.config.database.path}`);

      // Print stats
      this.printStats();

    } catch (error) {
      logger.error(`Failed to start storage server: ${error.message}`);
      await this.stop();
      process.exit(1);
    }
  }

  /**
   * Stop the storage server gracefully
   */
  async stop() {
    if (!this.running) {
      return;
    }

    logger.info('Stopping storage server...');

    try {
      // Stop WebSocket server
      if (this.wsServer) {
        this.wsServer.stop();
      }

      // Close database
      if (this.database) {
        this.database.close();
      }

      this.running = false;
      logger.info('Storage server stopped');
    } catch (error) {
      logger.error(`Error during shutdown: ${error.message}`);
    }
  }

  /**
   * Setup graceful shutdown handlers
   */
  setupShutdownHandlers() {
    const shutdown = async (signal) => {
      logger.info(`Received ${signal}, shutting down gracefully...`);
      await this.stop();
      process.exit(0);
    };

    process.on('SIGTERM', () => shutdown('SIGTERM'));
    process.on('SIGINT', () => shutdown('SIGINT'));

    process.on('uncaughtException', (error) => {
      logger.error(`Uncaught exception: ${error.message}`);
      logger.error(error.stack);
      this.stop().then(() => process.exit(1));
    });

    process.on('unhandledRejection', (reason, promise) => {
      logger.error(`Unhandled rejection at ${promise}: ${reason}`);
    });
  }

  /**
   * Print server statistics
   */
  printStats() {
    try {
      const dbStats = this.database.getStats();
      const queueStats = this.offlineQueue.getQueueStats();
      const connectionCount = this.wsServer.getConnectionCount();

      logger.info('=== Server Statistics ===');
      logger.info(`Database size: ${(dbStats.dbSize / 1024 / 1024).toFixed(2)} MB`);
      logger.info(`Units: ${dbStats.units}`);
      logger.info(`Tasks: ${dbStats.tasks} (${dbStats.disabledTasks} disabled)`);
      logger.info(`Queued messages: ${queueStats.pendingMessages} for ${queueStats.affectedUnits} units`);
      logger.info(`Active connections: ${connectionCount}`);
      logger.info('========================');
    } catch (error) {
      logger.error(`Error getting stats: ${error.message}`);
    }
  }

  /**
   * Get server status
   */
  getStatus() {
    return {
      running: this.running,
      serverId: this.config.server_id,
      connections: this.wsServer ? this.wsServer.getConnectionCount() : 0,
      stats: this.database ? this.database.getStats() : null
    };
  }
}

module.exports = StorageServer;
