/** The catalogue module of `en`: its shells load it before the page script. */

import { register } from '../language';
import catalogue from './en.json';

register(catalogue);
