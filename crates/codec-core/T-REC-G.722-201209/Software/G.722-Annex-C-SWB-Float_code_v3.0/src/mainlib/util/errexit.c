/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/

#include <stdio.h>
#include <stdlib.h>
#include "errexit.h"

void  error_exit( char *str )
{
  if ( str != NULL )
    fprintf( stderr, "%s\n", str );
  exit(1);
}
