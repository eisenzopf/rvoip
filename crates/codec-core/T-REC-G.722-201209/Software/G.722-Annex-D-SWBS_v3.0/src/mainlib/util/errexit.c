/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
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
