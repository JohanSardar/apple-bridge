import url from './emoji.json?url&no-inline';
import {loadCatalog} from '../catalog';
export type ReactionEmoji=readonly [string,string,string];
export function loadReactionEmoji(){return loadCatalog<readonly ReactionEmoji[]>(url);}
