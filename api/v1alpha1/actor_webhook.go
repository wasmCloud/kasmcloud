/*
Copyright 2023.

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
*/

package v1alpha1

import (
	"k8s.io/apimachinery/pkg/runtime"
	ctrl "sigs.k8s.io/controller-runtime"
	logf "sigs.k8s.io/controller-runtime/pkg/log"
	"sigs.k8s.io/controller-runtime/pkg/webhook"
	"sigs.k8s.io/controller-runtime/pkg/webhook/admission"
)

// log is for logging in this package.
var actorlog = logf.Log.WithName("actor-resource")

func (r *Actor) SetupWebhookWithManager(mgr ctrl.Manager) error {
	return ctrl.NewWebhookManagedBy(mgr).
		For(r).
		Complete()
}

// TODO(user): EDIT THIS FILE!  THIS IS SCAFFOLDING FOR YOU TO OWN!

//+kubebuilder:webhook:path=/mutate-kasmcloud-io-v1alpha1-actor,mutating=true,failurePolicy=fail,sideEffects=None,groups=kasmcloud.io,resources=actors,verbs=create;update,versions=v1alpha1,name=mactor.kb.io,admissionReviewVersions=v1

var _ webhook.Defaulter = &Actor{}

// Default implements webhook.Defaulter so a webhook will be registered for the type
func (r *Actor) Default() {
	actorlog.Info("default", "name", r.Name)

	// TODO(user): fill in your defaulting logic.
}

// TODO(user): change verbs to "verbs=create;update;delete" if you want to enable deletion validation.
//+kubebuilder:webhook:path=/validate-kasmcloud-io-v1alpha1-actor,mutating=false,failurePolicy=fail,sideEffects=None,groups=kasmcloud.io,resources=actors,verbs=create;update,versions=v1alpha1,name=vactor.kb.io,admissionReviewVersions=v1

var _ webhook.Validator = &Actor{}

// ValidateCreate implements webhook.Validator so a webhook will be registered for the type
func (r *Actor) ValidateCreate() (admission.Warnings, error) {
	actorlog.Info("validate create", "name", r.Name)

	// TODO(user): fill in your validation logic upon object creation.
	return nil, nil
}

// ValidateUpdate implements webhook.Validator so a webhook will be registered for the type
func (r *Actor) ValidateUpdate(old runtime.Object) (admission.Warnings, error) {
	actorlog.Info("validate update", "name", r.Name)

	// TODO(user): fill in your validation logic upon object update.
	return nil, nil
}

// ValidateDelete implements webhook.Validator so a webhook will be registered for the type
func (r *Actor) ValidateDelete() (admission.Warnings, error) {
	actorlog.Info("validate delete", "name", r.Name)

	// TODO(user): fill in your validation logic upon object deletion.
	return nil, nil
}
