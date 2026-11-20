/** The catalogue module of `de`: its shells load it before the page script. */

import { register } from '../language';
import catalogue from './de.json';

register(catalogue);
