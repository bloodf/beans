// Same public native request used by engine.ts; no second transport or phone-side execution.
import { request } from '../../modules/beans-core';
import { MemoryServiceAPI } from './memoryService';
export const memoryService = new MemoryServiceAPI(request);
