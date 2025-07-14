/* ITU G.722 3rd Edition (2012-09) */

/*
  ===========================================================================
   File: CONTROL.C                                       v.2.2 - 06.Dec.2006
  ===========================================================================

            ITU-T          STL          BASIC          OPERATORS

            CONTROL FLOW OPERATOR INTERNAL VARIABLE DECLARATIONS

   History:
   07 Nov 04   v2.0     Incorporation of new 32-bit / 40-bit / control
                        operators for the ITU-T Standard Tool Library as 
                        described in Geneva, 20-30 January 2004 WP 3/16 Q10/16
                        TD 11 document and subsequent discussions on the
                        wp3audio@yahoogroups.com email reflector.
   March 06   v2.1      Changed to improve portability.
   
  ============================================================================
*/

#include "stl.h"

#ifdef WMOPS
int funcId_where_last_call_to_else_occurred;
long funcid_total_wmops_at_last_call_to_else;
int call_occurred = 1;
#endif


/* end of file */
