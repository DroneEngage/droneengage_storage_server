const logger = require('./logger');
const database = require('./database');

// Message type constants (matching AndruavMessageTypes)
const CONST_TYPE_AndruavSystem_LoadTasks = 9001;
const CONST_TYPE_AndruavSystem_SaveTasks = 9002;
const CONST_TYPE_AndruavSystem_DeleteTasks = 9003;
const CONST_TYPE_AndruavSystem_DisableTasks = 9004;
const CONST_TYPE_AndruavSystem_UnitOnline = 9009; // Custom message for unit online event (aligned with shared AndruavMessageTypes enum)

class MessageHandlers {
  constructor(db, wsServer) {
    this.db = db;
    this.wsServer = wsServer;
  }

  /**
   * Extract Andruav payload from the ms field (falls back to the message itself for tests).
   */
  getPayload(message) {
    return (message && typeof message.ms === 'object' && message.ms !== null) ? message.ms : message;
  }

  /**
   * Echoes the request id (rid) from the incoming message, if present, so callers
   * (e.g. DBProxyClient on the comm server) can correlate concurrent requests/responses.
   */
  buildResponseEnvelope(message, mt, ms, success, error) {
    const envelope = { mt, ms, success, timestamp: Date.now() };
    if (message && message.rid != null) envelope.rid = message.rid;
    if (error !== undefined) envelope.error = error;
    return envelope;
  }

  /**
   * Handle LoadTasks message (9001)
   */
  async handleLoadTasks(connectionId, message) {
    const payload = this.getPayload(message);
    const { unitId } = payload;
    const connection = this.wsServer.getConnection(connectionId);

    try {
      logger.info(`LoadTasks request for unit ${unitId} from comm server ${connection.commServerId}`);

      // Get tasks from database
      const tasks = this.db.loadTasks(unitId);

      // Log access
      this.db.logAccess(unitId, 'load', 'task', null, connection.commServerId);

      // Send response - use 'ms' field for response data (matching existing system)
      this.wsServer.send(connectionId, this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_LoadTasks, {
        s: 'OK',
        tasks: tasks,
        unitId: unitId
      }, true));

      logger.info(`Loaded ${tasks.length} tasks for unit ${unitId}`);
    } catch (error) {
      logger.error(`Error loading tasks for unit ${unitId}: ${error.message}`);
      
      this.wsServer.send(connectionId, this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_LoadTasks, {
        s: 'ERROR:' + error.message,
        unitId: unitId
      }, false, error.message));
    }
  }

  /**
   * Handle SaveTasks message (9002)
   */
  async handleSaveTasks(connectionId, message) {
    const payload = this.getPayload(message);
    const { unitId, tasks } = payload;
    const connection = this.wsServer.getConnection(connectionId);

    try {
      logger.info(`SaveTasks request for unit ${unitId} with ${tasks.length} tasks`);

      // Ensure unit exists
      this.db.upsertUnit(unitId, null, connection.commServerId);

      // Save each task
      const savedTasks = [];
      for (const task of tasks) {
        const { taskId, name, data } = task;
        this.db.saveTask(taskId, unitId, name, data);
        savedTasks.push(taskId);
      }

      // Log access
      this.db.logAccess(unitId, 'save', 'task', savedTasks.join(','), connection.commServerId);

      // Send response
      this.wsServer.send(connectionId, this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_SaveTasks, {
        s: 'OK:save',
        savedTasks: savedTasks,
        unitId: unitId
      }, true));

      logger.info(`Saved ${savedTasks.length} tasks for unit ${unitId}`);
    } catch (error) {
      logger.error(`Error saving tasks for unit ${unitId}: ${error.message}`);
      
      this.wsServer.send(connectionId, this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_SaveTasks, {
        s: 'ERROR:' + error.message,
        unitId: unitId
      }, false, error.message));
    }
  }

  /**
   * Handle DeleteTasks message (9003)
   */
  async handleDeleteTasks(connectionId, message) {
    const payload = this.getPayload(message);
    const { unitId, taskIds } = payload;
    const connection = this.wsServer.getConnection(connectionId);

    try {
      logger.info(`DeleteTasks request for unit ${unitId} with ${taskIds.length} tasks`);

      // Delete each task
      const deletedTasks = [];
      for (const taskId of taskIds) {
        this.db.deleteTask(taskId);
        deletedTasks.push(taskId);
      }

      // Log access
      this.db.logAccess(unitId, 'delete', 'task', deletedTasks.join(','), connection.commServerId);

      // Send response
      this.wsServer.send(connectionId, this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_DeleteTasks, {
        s: 'OK:del',
        deletedTasks: deletedTasks,
        unitId: unitId
      }, true));

      logger.info(`Deleted ${deletedTasks.length} tasks for unit ${unitId}`);
    } catch (error) {
      logger.error(`Error deleting tasks for unit ${unitId}: ${error.message}`);
      
      this.wsServer.send(connectionId, this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_DeleteTasks, {
        s: 'ERROR:' + error.message,
        unitId: unitId
      }, false, error.message));
    }
  }

  /**
   * Handle DisableTasks message (9004)
   */
  async handleDisableTasks(connectionId, message) {
    const payload = this.getPayload(message);
    const { unitId, taskIds } = payload;
    const connection = this.wsServer.getConnection(connectionId);

    try {
      logger.info(`DisableTasks request for unit ${unitId} with ${taskIds.length} tasks`);

      // Disable each task
      const disabledTasks = [];
      for (const taskId of taskIds) {
        this.db.disableTask(taskId);
        disabledTasks.push(taskId);
      }

      // Log access
      this.db.logAccess(unitId, 'disable', 'task', disabledTasks.join(','), connection.commServerId);

      // Send response
      this.wsServer.send(connectionId, this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_DisableTasks, {
        s: 'OK:disable',
        disabledTasks: disabledTasks,
        unitId: unitId
      }, true));

      logger.info(`Disabled ${disabledTasks.length} tasks for unit ${unitId}`);
    } catch (error) {
      logger.error(`Error disabling tasks for unit ${unitId}: ${error.message}`);
      
      this.wsServer.send(connectionId, this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_DisableTasks, {
        s: 'ERROR:' + error.message,
        unitId: unitId
      }, false, error.message));
    }
  }

  /**
   * Handle UnitOnline message (9010) - Trigger offline queue processing
   */
  async handleUnitOnline(connectionId, message) {
    const payload = this.getPayload(message);
    const { unitId } = payload;
    const connection = this.wsServer.getConnection(connectionId);

    try {
      logger.info(`Unit online event for unit ${unitId} from comm server ${connection.commServerId}`);

      // Process offline queue for this unit
      const OfflineQueue = require('./offlineQueue');
      const offlineQueue = new OfflineQueue(this.db, this.wsServer);
      
      const result = await offlineQueue.processQueueForUnit(unitId, connectionId);

      // Send response
      this.wsServer.send(connectionId, this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_UnitOnline, {
        s: 'OK',
        unitId: unitId,
        processed: result.processed,
        failed: result.failed
      }, true));

      logger.info(`Unit online processed for unit ${unitId}: ${result.processed} messages delivered, ${result.failed} failed`);
    } catch (error) {
      logger.error(`Error processing unit online for unit ${unitId}: ${error.message}`);
      
      this.wsServer.send(connectionId, this.buildResponseEnvelope(message, CONST_TYPE_AndruavSystem_UnitOnline, {
        s: 'ERROR:' + error.message,
        unitId: unitId
      }, false, error.message));
    }
  }

  /**
   * Handle message by numeric type (mt field)
   */
  async handleMessageByType(connectionId, message) {
    const messageType = message.mt;

    switch (messageType) {
      case CONST_TYPE_AndruavSystem_LoadTasks:
        await this.handleLoadTasks(connectionId, message);
        break;
      case CONST_TYPE_AndruavSystem_SaveTasks:
        await this.handleSaveTasks(connectionId, message);
        break;
      case CONST_TYPE_AndruavSystem_DeleteTasks:
        await this.handleDeleteTasks(connectionId, message);
        break;
      case CONST_TYPE_AndruavSystem_DisableTasks:
        await this.handleDisableTasks(connectionId, message);
        break;
      case CONST_TYPE_AndruavSystem_UnitOnline:
        await this.handleUnitOnline(connectionId, message);
        break;
      default:
        logger.warn(`Unknown message type: ${messageType}`);
        this.wsServer.sendError(connectionId, `Unknown message type: ${messageType}`);
    }
  }

  /**
   * Get handlers map - numeric type mapping
   */
  getHandlers() {
    return {
      [CONST_TYPE_AndruavSystem_LoadTasks]: this.handleLoadTasks.bind(this),
      [CONST_TYPE_AndruavSystem_SaveTasks]: this.handleSaveTasks.bind(this),
      [CONST_TYPE_AndruavSystem_DeleteTasks]: this.handleDeleteTasks.bind(this),
      [CONST_TYPE_AndruavSystem_DisableTasks]: this.handleDisableTasks.bind(this),
      [CONST_TYPE_AndruavSystem_UnitOnline]: this.handleUnitOnline.bind(this)
    };
  }
}

module.exports = MessageHandlers;
