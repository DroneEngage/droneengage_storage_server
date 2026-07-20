const logger = require('./logger');
const database = require('./database');
const serverConfig = require('../js_serverConfig.js');

class OfflineQueue {
  constructor(db, wsServer) {
    const config = serverConfig.m_configuration;
    this.db = db;
    this.wsServer = wsServer;
    this.config = config;
    this.maxQueueSize = config.queue.maxQueueSize;
    this.retryInterval = config.queue.retryInterval;
    this.processing = false;
  }

  /**
   * Add message to offline queue for a unit
   */
  enqueue(unitId, messageType, messageData, taskId = null, priority = 0) {
    try {
      // Check queue size
      const queueSize = this.db.prepare(`
        SELECT COUNT(*) as count FROM offline_queue 
        WHERE unit_id = ? AND status = 'pending'
      `).get(unitId).count;

      if (queueSize >= this.maxQueueSize) {
        logger.warn(`Queue full for unit ${unitId}, rejecting message`);
        return { success: false, error: 'Queue full' };
      }

      // Add to queue
      this.db.addToQueue(unitId, messageType, messageData, taskId, priority);
      
      logger.debug(`Enqueued message for unit ${unitId}, queue size: ${queueSize + 1}`);
      return { success: true };
    } catch (error) {
      logger.error(`Error enqueuing message for unit ${unitId}: ${error.message}`);
      return { success: false, error: error.message };
    }
  }

  /**
   * Process queued messages for a unit when it comes online
   */
  async processQueueForUnit(unitId, connectionId) {
    try {
      logger.info(`Processing offline queue for unit ${unitId}`);

      const messages = this.db.getQueuedMessages(unitId, 50);
      
      if (messages.length === 0) {
        logger.info(`No queued messages for unit ${unitId}`);
        return { success: true, processed: 0 };
      }

      let processed = 0;
      let failed = 0;

      for (const message of messages) {
        try {
          // Send message to unit via comm server
          const success = await this.deliverMessage(connectionId, message);
          
          if (success) {
            this.db.markMessageDelivered(message.id);
            processed++;
          } else {
            failed++;
            logger.warn(`Failed to deliver message ${message.id} for unit ${unitId}`);
          }
        } catch (error) {
          logger.error(`Error delivering message ${message.id}: ${error.message}`);
          failed++;
        }
      }

      logger.info(`Processed ${processed} messages for unit ${unitId}, ${failed} failed`);
      
      // Cleanup delivered messages
      this.db.cleanupDeliveredMessages();

      return { success: true, processed, failed };
    } catch (error) {
      logger.error(`Error processing queue for unit ${unitId}: ${error.message}`);
      return { success: false, error: error.message };
    }
  }

  /**
   * Deliver a single queued message
   */
  async deliverMessage(connectionId, message) {
    try {
      const messageData = {
        type: 'queued_message',
        messageType: message.message_type,
        data: message.message_data,
        taskId: message.task_id,
        queueId: message.id,
        timestamp: Date.now()
      };

      this.wsServer.send(connectionId, messageData);
      return true;
    } catch (error) {
      logger.error(`Error delivering message: ${error.message}`);
      return false;
    }
  }

  /**
   * Get queue status for a unit
   */
  getQueueStatus(unitId) {
    try {
      const status = this.db.prepare(`
        SELECT 
          COUNT(*) as total,
          SUM(CASE WHEN status = 'pending' THEN 1 ELSE 0 END) as pending,
          SUM(CASE WHEN status = 'delivered' THEN 1 ELSE 0 END) as delivered
        FROM offline_queue
        WHERE unit_id = ?
      `).get(unitId);

      return status;
    } catch (error) {
      logger.error(`Error getting queue status for unit ${unitId}: ${error.message}`);
      return { total: 0, pending: 0, delivered: 0 };
    }
  }

  /**
   * Get overall queue statistics
   */
  getQueueStats() {
    try {
      const stats = this.db.prepare(`
        SELECT 
          COUNT(*) as totalMessages,
          SUM(CASE WHEN status = 'pending' THEN 1 ELSE 0 END) as pendingMessages,
          COUNT(DISTINCT unit_id) as affectedUnits
        FROM offline_queue
        WHERE status = 'pending'
      `).get();

      return stats;
    } catch (error) {
      logger.error(`Error getting queue stats: ${error.message}`);
      return { totalMessages: 0, pendingMessages: 0, affectedUnits: 0 };
    }
  }

  /**
   * Clear queue for a unit
   */
  clearQueue(unitId) {
    try {
      const result = this.db.prepare(`
        DELETE FROM offline_queue WHERE unit_id = ?
      `).run(unitId);

      logger.info(`Cleared ${result.changes} messages from queue for unit ${unitId}`);
      return { success: true, cleared: result.changes };
    } catch (error) {
      logger.error(`Error clearing queue for unit ${unitId}: ${error.message}`);
      return { success: false, error: error.message };
    }
  }

  /**
   * Start background cleanup job
   */
  startCleanupJob() {
    // Run cleanup every hour
    setInterval(() => {
      this.db.cleanupDeliveredMessages();
    }, 60 * 60 * 1000);

    logger.info('Offline queue cleanup job started');
  }
}

module.exports = OfflineQueue;
