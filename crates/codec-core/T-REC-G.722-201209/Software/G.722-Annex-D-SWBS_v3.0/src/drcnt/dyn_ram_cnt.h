/* ITU G.722 3rd Edition (2012-09) */


/*dynamic RAM counting tool                */
/*version 1.2                              */
/*23th March 2010                        */
/*contact: balazs.kovesi@orange-ftgroup.com*/

/*****************************
This tool is developped to help the estimation of the dynamic memory RAM 
complexity of a software written ic C. If all functions used in the software 
are instrumented using this tool, it gives the highest memory allocation 
occurred during the execution. A detailed summary is printed in the file 
dyn_ram_max_info.txt, containing the max dynamic ram usage in bytes and the 
call graph of the worst case path.


How to use this tool

1.)
Define the preprocessor directive DYN_RAM_CNT

2.)
Define the following variables in a global position before the main function

#ifdef DYN_RAM_CNT
int           dyn_ram_level_cnt;
unsigned long *dyn_ram_table_ptr;
unsigned long dyn_ram_table[DYN_RAM_MAX_LEVEL];
char          dyn_ram_name_table[DYN_RAM_MAX_LEVEL][DYN_RAM_MAX_NAME_LENGTH];
unsigned long dyn_ram_current_value;
unsigned long dyn_ram_max_value;
unsigned long dyn_ram_max_counter;
#endif 

int main(int argc, char *argv[])
{
...

3.) at the beginning of the main function call the init function:

#ifdef DYN_RAM_CNT
  DYN_RAM_INIT();
#endif 


4.)
include in each .c source file the dyn_ram_cnt.h file (this file):

#ifdef DYN_RAM_CNT
#include "dyn_ram_cnt.h"
#endif

5.)
At the beginning of each function (or at the beginning of blocks where new dynamic 
RAM is allocated or after a dynamic memory allocation using malloc or calloc) copy 
this part and fill in the good numbers for each type of variables:

#ifdef DYN_RAM_CNT
  {
    unsigned long drsize = 0;
    drsize += (UWord32) (0 * SIZE_Word16);
    drsize += (UWord32) (0 * SIZE_Word32);
    drsize += (UWord32) (0 * SIZE_Ptr);
    DYN_RAM_PUSH(drsize,"identifier name");
  }
#endif 

6.)

At the end of each function (or at the end of blocks where new dynamic 
RAM was allocated or after the free of a dynamic memory allocation using 
free) copy this part and fill in the good numbers for each type of variables:

#ifdef DYN_RAM_CNT
  {
    DYN_RAM_POP();
  }
#endif 

7.) Optional :
At the end of the main function call the report function:

#ifdef DYN_RAM_CNT
  DYN_RAM_REPORT();
#endif 

This prints out on the screen the found maximum dynamic RAM usage in bytes 
and some other information that can be useful like the verification result for
PUSH-POP pairs or an identification number that help to put a conditional
breakpoint to stop the execution at the moment when the worst case situation happen.
In this way this tool gives another possibility to check in which configuration the max 
memory need occurred. To use the facility, one must put a conditional break point in 
dyn_ram_cnt.h in the DYN_RAM_PUSH function (see comments in the function).
The breakpoint Condition is : 
dyn_ram_max_counter == <the number printed out at the end of the first run>. 
After putting this breakpoint, run again executable, when break happens, check the 
call stack and the data in the global dyn_ram_table array.

*/



#ifdef DYN_RAM_CNT
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define DYN_RAM_MAX_LEVEL (200)
#define DYN_RAM_MAX_NAME_LENGTH (50)

#define SIZE_Word16  2
#define SIZE_Word32  4
#define SIZE_Ptr     2

extern int           dyn_ram_level_cnt;
extern unsigned long *dyn_ram_table_ptr;
extern unsigned long dyn_ram_table[DYN_RAM_MAX_LEVEL];
extern char          dyn_ram_name_table[DYN_RAM_MAX_LEVEL][DYN_RAM_MAX_NAME_LENGTH];
extern unsigned long dyn_ram_current_value;
extern unsigned long dyn_ram_max_value;
extern unsigned long dyn_ram_max_counter;

#ifdef MAIN_ROUTINE
static void DYN_RAM_INIT()      
{                                             
	dyn_ram_level_cnt = 0;                      
	dyn_ram_current_value = (unsigned long) 0;        
	dyn_ram_table_ptr = &dyn_ram_table[0];      
	dyn_ram_max_value = 0;                      
	dyn_ram_max_counter = 0;                    
}
#endif

static void DYN_RAM_PUSH(unsigned long size, char * dyn_ram_name)  
{    
	int i;
	*dyn_ram_table_ptr = size; 

	i = 0;
	while(dyn_ram_name[i] != 0)
	{ 
		dyn_ram_name_table[dyn_ram_level_cnt][i]=dyn_ram_name[i]; 
		i++; 
	}
	for(; i < DYN_RAM_MAX_NAME_LENGTH; i++) 
	{
		dyn_ram_name_table[dyn_ram_level_cnt][i]=0; 
	}

	dyn_ram_current_value += *dyn_ram_table_ptr;         
	if (dyn_ram_current_value > dyn_ram_max_value)       
	{                            
		FILE * drfp;
		dyn_ram_max_counter++;                                   
		dyn_ram_max_value = dyn_ram_current_value;  
/*you can put a conditional breakpoint on the above line, when dyn_ram_max_counter equals to the printed number of the first run*/        
		drfp = fopen("dyn_ram_max_info.txt","w");
		if(drfp != NULL)
		{
			fprintf(drfp,"Max dynamic ram usage : %ld bytes\n", dyn_ram_max_value);
			fprintf(drfp,"Details of worst case path :\n");
			for(i = 0; i <= (int)dyn_ram_level_cnt; i++)
			{
				fprintf(drfp,"%3d : %6ld bytes in %s\n", i+1, dyn_ram_table[i], dyn_ram_name_table[i]);
			}
			fclose(drfp);
		}
		else
		{
			printf("\n WARNING : cannot open dyn_ram_max_info.txt for writing\n");
		}
	}                                          

	dyn_ram_table_ptr++; 
	dyn_ram_level_cnt++;  /* used for final verification of PUSH-POP pairs*/                     
}

static void DYN_RAM_POP()       
{             
	int i;
	dyn_ram_table_ptr--;                    
	dyn_ram_level_cnt--;                  

	dyn_ram_current_value -= *dyn_ram_table_ptr; 

	*dyn_ram_table_ptr = 0;  
	for(i=0; i < DYN_RAM_MAX_NAME_LENGTH; i++) 
	{
		dyn_ram_name_table[dyn_ram_level_cnt][i]=0; 
	}
}

#ifdef MAIN_ROUTINE
static void DYN_RAM_REPORT()                                                        
{                                                                             
	printf("\n**************************************************************************************\n");  
	printf("* Max dynamic RAM usage : %ld Bytes", dyn_ram_max_value);          
	printf("\n* at max number : %ld (put conditional breakpoint to this value in DYN_RAM_PUSH)", dyn_ram_max_counter);       
	printf("\n* See also dyn_ram_max_info.txt for memory report and worst case path details ");       
	printf("\n* Verification before exit : %ld Bytes; %d levels (both must be 0, otherwise PUSH-POP mismatch)", dyn_ram_current_value, dyn_ram_level_cnt);     
	printf("\n**************************************************************************************\n");  
}
#endif

#endif /* ifdef DYN_RAM_CNT */

