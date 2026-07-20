const logger = require('./logger');
const database = require('./database');

// Message type constants (matching AndruavMessageTypes)
const CONST_TYPE_AndruavSystem_LoadTasks = 9001;
const CONST_TYPE_AndruavSystem_SaveTasks = 9002;
const CONST_TYPE_AndruavSystem_DeleteTasks = 9003;
const CONST_TYPE_AndruavSystem_DisableTasks = 9004;

class MessageHandlers {
  constructor(db, wsServer) {
    this.db = db;
    this.wsServer = wsServer;
  }

  /**
   * Handle LoadTasks message (9001)
   */
  async handleLoadTasks(connectionId, message) {
    const { unitId } = message;
    const connection = this.wsServer.getConnection(connectionId);

    try {
      logger.info(`LoadTasks request for unit ${unitId} from comm server ${connection.commServerId}`);

      // Get tasks from database
      const tasks = this.db.loadTasks(unitId);

      // Log access
      this.db.logAccess(unitId, 'load', 'task', null, connection.commServerId);

      // Send response
      this.wsServer.send(connectionId, {
        type: 'LoadTasks_response',
        unitId: unitId,
        tasks: tasks,
        success: true,
        timestamp: Date.now()
      });

      logger.info(`Loaded ${tasks.length} tasks for unit ${unitId}`);
    } catch (error) {
      logger.error(`Error loading tasks for unit ${unitId}: ${error.message}`);
      
      this.wsServer.send(connectionId, {
        type: 'LoadTasks_response',
        unitId: unitId,
        success: false,
        error: error.message,
        timestamp: Date.now()
      });
    }
  }

  /**
   * Handle SaveTasks message (9002)
   */
  async handleSaveTasks(connectionId, message) {
    const { unitId, tasks } = message;
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
      this.wsServer.send(connectionId, {
        type: 'SaveTasks_response',
        unitId: unitId,
        savedTasks: savedTasks,
        success: true,
        timestamp: Date.now()
      });

      logger.info(`Saved ${savedTasks.length} tasks for unit ${unitId}`);
    } catch (error) {
      logger.error(`Error saving tasks for unit ${unitId}: ${error.message}`);
      
      this.wsServer.send(connectionId, {
        type: 'SaveTasks_response',
        unitId: unitId,
        success: false,
        error: error.message,
        timestamp: Date.now()
      });
    }
  }

  /**
   * Handle DeleteTasks message (9003)
   */
  async handleDeleteTasks(connectionId, message) {
    const { unitId, taskIds } = message;
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
      this.wsServer.send(connectionId, {
        type: 'DeleteTasks_response',
        unitId: unitId,
        deletedTasks: deletedTasks,
        success: true,
        timestamp: Date.now()
      });

      logger.info(`Deleted ${deletedTasks.length} tasks for unit ${unitId}`);
    } catch (error) {
      logger.error(`Error deleting tasks for unit ${unitId}: ${error.message}`);
      
      this.wsServer.send(connectionId, {
        type: 'DeleteTasks_response',
        unitId: unitId,
        success: false,
        error: error.message,
        timestamp: Date.now()
      });
    }
  }

  /**
   * Handle DisableTasks message (9004)
   */
  async handleDisableTasks(connectionId, message) {
    const { unitId, taskIds } = message;
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
      this.wsServer.send(connectionId, {
        type: 'DisableTasks_response',
        unitId: unitId,
        disabledTasks: disabledTasks,
        success: true,
        timestamp: Date.now()
      });

      logger.info(`Disabled ${disabledTasks.length} tasks for unit ${unitId}`);
    } catch (error) {
      logger.error(`Error disabling tasks for unit ${unitId}: ${error.message}`);
      
      this.wsServer.send(connectionId, {
        type: 'DisableTasks_response',
        unitId: unitId,
        success: false,
        error: error.message,
        timestamp: Date.now()
      });
    }
  }

  /**
   * Handle generic Andruav system message
   */
  async handleAndruavSystemMessage(connectionId, message) {
    const { messageType } = message;

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
      default:
        logger.warn(`Unknown Andruav system message type: ${messageType}`);
        this.wsServer.sendError(connectionId, `Unknown message type: ${messageType}`);
    }
  }

  /**
   * Get handlers map
   */
  getHandlers() {
    return {
      'AndruavSystem': this.handleAndruavSystemMessage.bind(this),
      'LoadTasks': this.handleLoadTasks.bind(this),
      'SaveTasks': this.handleSaveTasks.bind(this),
      'DeleteTasks': this.handleDeleteTasks.bind(this),
      'DisableTasks': this.handleDisableTasks.bind(this)
    };
  }
}

module.exports = MessageHandlers;
