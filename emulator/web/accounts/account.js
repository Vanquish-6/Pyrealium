'use strict';
const form=document.querySelector('#register'),status=document.querySelector('#status'),button=form.querySelector('button');
form.addEventListener('submit',async(event)=>{
  event.preventDefault();
  if(!form.reportValidity())return;
  const password=form.elements.password.value;
  if(password!==form.elements.confirm.value){status.textContent='The passwords do not match.';form.elements.confirm.focus();return;}
  button.disabled=true;status.textContent='Creating your account…';
  try{
    const response=await fetch('/api/register',{method:'POST',headers:{'Content-Type':'application/json'},credentials:'omit',body:JSON.stringify({account:form.elements.account.value.trim().toLowerCase(),password})});
    const result=await response.json();
    status.textContent=result.message||'The service could not complete your request.';
    if(response.ok){form.reset();button.textContent='Account created';}else{button.disabled=false;}
  }catch{status.textContent='Could not reach the account service. Please try again.';button.disabled=false;}
});
